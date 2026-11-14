//! «Живые обои» — видео, анимация и звук на рабочем столе Windows.
#![cfg_attr(
    all(windows, not(debug_assertions), not(test)),
    windows_subsystem = "windows"
)]
// Вне Windows собирается только переносимая часть — для тестов.
#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(windows)]
mod desktop;
#[cfg(windows)]
mod engine;
mod library;
mod log;
mod mpv;
mod online;
#[cfg(windows)]
mod system;
#[cfg(windows)]
mod theme;
mod thumbs;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod ui;

#[cfg(not(windows))]
fn main() {
    eprintln!("«Живые обои» работают только в Windows.");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() -> eframe::Result {
    use eframe::egui;

    // Отчёт для поиска неполадок: `zhivye-oboi.exe --diagnose отчёт.txt`.
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--diagnose") {
        let out = args
            .get(pos + 1)
            .cloned()
            .unwrap_or_else(|| "diagnose.txt".into());
        let report = diagnose();
        let _ = std::fs::write(&out, &report);
        std::process::exit(if report.contains("mpv: ошибка") {
            1
        } else {
            0
        });
    }

    install_panic_hook();
    log::write(&format!(
        "запуск {} {:?}",
        env!("CARGO_PKG_VERSION"),
        &args[1..]
    ));

    // Второй экземпляр не нужен: два набора окон за значками мешают друг
    // другу. Если программа уже запущена — показываем её окно.
    if system::signal_running_instance() {
        log::write("уже запущена — показываем её окно");
        return Ok(());
    }

    // Отрисовка: сначала wgpu (DirectX 12 / Vulkan). Если на этом компьютере
    // он не поднимается, перезапускаемся с OpenGL (`--opengl`): второй цикл
    // событий winit в том же процессе создать нельзя.
    let opengl = args.iter().any(|a| a == "--opengl");

    let tray_start = std::env::args().any(|a| a == "--tray");
    let icon = egui::IconData {
        rgba: tray::icon_rgba(64),
        width: 64,
        height: 64,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(ui::TITLE)
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([720.0, 520.0])
            .with_icon(icon)
            .with_visible(!tray_start),
        renderer: if opengl {
            eframe::Renderer::Glow
        } else {
            eframe::Renderer::Wgpu
        },
        ..Default::default()
    };
    let result = eframe::run_native(
        ui::TITLE,
        options,
        Box::new(|cc| Ok(Box::new(ui::App::new(cc)))),
    );
    engine::shutdown();
    if let Err(e) = &result {
        let renderer = if opengl { "OpenGL" } else { "wgpu" };
        log::write(&format!("окно не открылось ({renderer}): {e}"));
        if !opengl {
            if let Ok(exe) = std::env::current_exe() {
                let mut child_args: Vec<String> = args[1..].to_vec();
                child_args.push("--opengl".into());
                if std::process::Command::new(exe)
                    .args(&child_args)
                    .spawn()
                    .is_ok()
                {
                    log::write("перезапуск с OpenGL");
                    return Ok(());
                }
            }
        }
        system::error_box(&format!(
            "Не удалось открыть окно программы.\n\n{e}\n\n\
             Обновите драйвер видеокарты. Подробности — в журнале:\n{}",
            log::path().display()
        ));
    }
    result
}

/// Падение программы не должно быть молчаливым: пишем в журнал и
/// показываем окно с текстом ошибки.
#[cfg(windows)]
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::force_capture();
        log::write(&format!("сбой: {info}\n{bt}"));
        system::error_box(&format!(
            "Программа столкнулась с ошибкой и будет закрыта.\n\n{info}\n\nЖурнал: {}",
            log::path().display()
        ));
    }));
}

/// Проверяет mpv, мониторы и слой рабочего стола и на пару секунд
/// ставит тестовую картинку на основной монитор.
#[cfg(windows)]
fn diagnose() -> String {
    use std::fmt::Write;
    let mut r = String::new();
    let _ = writeln!(r, "Живые обои {}", env!("CARGO_PKG_VERSION"));
    let tail = log::tail(30);
    if !tail.is_empty() {
        let _ = writeln!(r, "--- журнал ---\n{tail}\n--------------");
    }
    match mpv::Player::new(&[("vo", "null"), ("ao", "null"), ("config", "no")]) {
        Ok(p) => {
            let _ = writeln!(r, "mpv: {}", p.get("mpv-version").unwrap_or_default());
            let _ = writeln!(r, "ffmpeg: {}", p.get("ffmpeg-version").unwrap_or_default());
        }
        Err(e) => {
            let _ = writeln!(r, "mpv: ошибка: {e}");
            return r;
        }
    }
    let monitors = desktop::monitors();
    for m in &monitors {
        let _ = writeln!(
            r,
            "монитор {}: {}×{} в ({}, {}){}",
            m.device,
            m.width(),
            m.height(),
            m.rect.left,
            m.rect.top,
            if m.primary { ", основной" } else { "" }
        );
    }
    let Some(layer) = desktop::find_layer() else {
        let _ = writeln!(r, "слой рабочего стола: не найден (проводник не запущен?)");
        return r;
    };
    let _ = writeln!(r, "слой рабочего стола: {}", layer.describe());
    let Some(m) = monitors.first() else { return r };
    let Some(host) = desktop::create_host(layer, m) else {
        let _ = writeln!(r, "окно обоев: не создано");
        return r;
    };
    let opts = mpv::wallpaper_options(
        Some((host.0 as usize).to_string()),
        library::Kind::Video,
        library::Fit::Fill,
        true,
        0,
        false,
    );
    let refs: Vec<(&str, &str)> = opts.iter().map(|(k, v)| (*k, v.as_str())).collect();
    match mpv::Player::new(&refs).and_then(|p| {
        p.load("av://lavfi:testsrc2=size=1280x720:rate=30")?;
        Ok(p)
    }) {
        Ok(p) => {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while std::time::Instant::now() < until {
                pump_messages();
                p.drain_events();
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
            let _ = writeln!(
                r,
                "проигрывание: vo={} hwdec={} кадр={}",
                p.get("current-vo").unwrap_or_else(|| "нет".into()),
                p.get("hwdec-current").unwrap_or_else(|| "нет".into()),
                p.get("estimated-frame-number")
                    .unwrap_or_else(|| "?".into()),
            );
        }
        Err(e) => {
            let _ = writeln!(r, "проигрывание: ошибка: {e}");
        }
    }
    desktop::destroy(host);
    desktop::refresh_system_wallpaper();
    r
}

#[cfg(windows)]
fn pump_messages() {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
