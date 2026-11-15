//! Vibewall — живые обои для Windows: видео, анимация и звук на рабочем столе.
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
mod sources;
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
    eprintln!("Vibewall работает только в Windows.");
    std::process::exit(1);
}

#[cfg(windows)]
fn main() -> eframe::Result {
    use eframe::egui;

    // Отчёт для поиска неполадок: `vibewall.exe --diagnose отчёт.txt`.
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

    library::migrate_old_dir();
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

    // Отрисовка: сначала OpenGL, запасной вариант — wgpu (DirectX 12 /
    // Vulkan, в том числе программный WARP без видеокарты). Сбой отрисовки
    // часто не ошибка, а зависание или падение в драйвере, поэтому выбор
    // запоминается в renderer.txt, а сторож переключает способ, если окно
    // не появилось. Второй цикл событий winit в одном процессе создать
    // нельзя — переключение только перезапуском.
    let tray_start = args.iter().any(|a| a == "--tray");
    let opengl = choose_opengl(&args);
    ui::USING_OPENGL.store(opengl, std::sync::atomic::Ordering::Relaxed);
    log::write(if opengl {
        "отрисовка: OpenGL"
    } else {
        "отрисовка: wgpu"
    });
    if !tray_start {
        set_renderer_state(if opengl {
            "opengl-starting"
        } else {
            "wgpu-starting"
        });
        start_watchdog(opengl, args[1..].to_vec());
    }
    let (rgba, width, height) = tray::icon_rgba(256);
    let icon = egui::IconData {
        rgba,
        width,
        height,
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
        if switch_renderer(opengl, &args[1..]) {
            return Ok(());
        }
        system::error_box(&format!(
            "Не удалось открыть окно программы.\n\n{e}\n\n\
             Обновите драйвер видеокарты. Подробности — в журнале:\n{}",
            log::path().display()
        ));
    }
    result
}

fn renderer_file() -> std::path::PathBuf {
    log::path().with_file_name("renderer.txt")
}

#[cfg(windows)]
pub fn set_renderer_state(state: &str) {
    let _ = std::fs::write(renderer_file(), state);
}

/// `--opengl` / `--wgpu` — явный выбор. Иначе — что сработало в прошлый
/// раз; если прошлый запуск так и не показал окно (`*-starting`), берём
/// другой способ. По умолчанию OpenGL.
#[cfg(windows)]
fn choose_opengl(args: &[String]) -> bool {
    if args.iter().any(|a| a == "--opengl") {
        return true;
    }
    if args.iter().any(|a| a == "--wgpu") {
        return false;
    }
    let state = std::fs::read_to_string(renderer_file()).unwrap_or_default();
    match state.trim() {
        "wgpu" => false,
        "opengl-starting" => {
            log::write("прошлый запуск с OpenGL не показал окно — пробуем wgpu");
            false
        }
        "wgpu-starting" => {
            log::write("прошлый запуск с wgpu не показал окно — пробуем OpenGL");
            true
        }
        _ => true,
    }
}

/// Перезапуск с другим способом отрисовки. Только один раз за цепочку
/// (`--fallback`), чтобы не перезапускаться по кругу.
#[cfg(windows)]
fn switch_renderer(opengl: bool, args: &[String]) -> bool {
    if args.iter().any(|a| a == "--fallback") {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let mut child: Vec<String> = args
        .iter()
        .filter(|a| *a != "--wgpu" && *a != "--opengl")
        .cloned()
        .collect();
    child.push(if opengl { "--wgpu" } else { "--opengl" }.into());
    child.push("--fallback".into());
    let ok = std::process::Command::new(exe).args(&child).spawn().is_ok();
    if ok {
        log::write(if opengl {
            "перезапуск с wgpu"
        } else {
            "перезапуск с OpenGL"
        });
    }
    ok
}

/// Сторож: если окно не нарисовалось за 15 секунд, отрисовка, скорее
/// всего, зависла в драйвере — перезапускаемся другим способом, а если
/// уже пробовали оба, честно сообщаем.
#[cfg(windows)]
fn start_watchdog(opengl: bool, args: Vec<String>) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(15));
        if ui::FIRST_FRAME.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let renderer = if opengl { "OpenGL" } else { "wgpu" };
        log::write(&format!("окно с {renderer} не появилось за 15 с"));
        set_renderer_state(if opengl {
            "opengl-starting"
        } else {
            "wgpu-starting"
        });
        if switch_renderer(opengl, &args) {
            std::process::exit(3);
        }
        system::error_box(&format!(
            "Окно программы не открылось: видеокарта не отвечает ни через OpenGL, \
             ни через DirectX/Vulkan.\n\nОбновите драйвер видеокарты. Журнал:\n{}",
            log::path().display()
        ));
        std::process::exit(4);
    });
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
    let _ = writeln!(r, "Vibewall {}", env!("CARGO_PKG_VERSION"));
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
