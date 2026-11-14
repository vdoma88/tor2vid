//! Значок в трее: открыть панель, пауза, выход. Обработчики вызываются в
//! главном потоке (их доставляет скрытое окно tray-icon), поэтому движок
//! доступен прямо из них.

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::engine;
use crate::system::Panel;

/// Значок 32×32: квадрат цвета акцента со скруглением и белым «play».
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let mut px = vec![0u8; (size * size * 4) as usize];
    let s = size as f32;
    let r = s * 0.22;
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Скруглённый квадрат.
            let dx = (r - fx).max(fx - (s - r)).max(0.0);
            let dy = (r - fy).max(fy - (s - r)).max(0.0);
            let corner = (dx * dx + dy * dy).sqrt();
            let alpha = (r - corner + 0.5).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                continue;
            }
            // Треугольник «play».
            let (ax, ay, h) = (s * 0.38, s * 0.28, s * 0.44);
            let t = (fy - ay) / h;
            let inside = (0.0..=1.0).contains(&t)
                && fx >= ax
                && fx <= ax + (1.0 - (2.0 * t - 1.0).abs()) * s * 0.36;
            let (cr, cg, cb) = if inside {
                (255, 255, 255)
            } else {
                (0x5e, 0x6a, 0xd2)
            };
            let i = ((y * size + x) * 4) as usize;
            px[i] = cr;
            px[i + 1] = cg;
            px[i + 2] = cb;
            px[i + 3] = (alpha * 255.0) as u8;
        }
    }
    px
}

pub fn create(panel: Panel, ctx: eframe::egui::Context) -> Option<TrayIcon> {
    let menu = Menu::new();
    let open = MenuItem::new("Открыть «Живые обои»", true, None);
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

    let icon = Icon::from_rgba(icon_rgba(32), 32, 32).ok()?;
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("Живые обои")
        .with_icon(icon)
        .build()
        .ok()
}
