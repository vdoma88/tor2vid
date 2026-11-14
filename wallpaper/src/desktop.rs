//! Окно за значками рабочего стола.
//!
//! Приём тот же, что в Lively, weebp и других открытых проектах: Progman
//! получает недокументированное сообщение 0x052C и создаёт слой WorkerW
//! между обоями и значками. Дальше два варианта раскладки:
//!
//! * Windows 10 и 11 до 23H2 — значки (SHELLDLL_DefView) уезжают в
//!   отдельное верхнее окно WorkerW, а за ним появляется пустое WorkerW.
//!   Наше окно становится его дочерним.
//! * Windows 11 24H2 и новее («raised desktop») — значки остаются внутри
//!   Progman, WorkerW тоже дочернее у Progman. Наше окно делается
//!   слоистым дочерним окном Progman и ставится сразу под значками:
//!   обычное дочернее окно DWM в этом дереве не рисует.

use std::sync::Once;

use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, EnumDisplayMonitors, FillRect, GetMonitorInfoW, GetStockObject,
    MapWindowPoints, BLACK_BRUSH, HBRUSH, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// Windows 11 24H2+: родитель — Progman, вставляем под значки.
    Raised { progman: HWND, icons: HWND },
    /// Классическая раскладка: родитель — пустое WorkerW за значками.
    Classic { workerw: HWND },
}

impl Layer {
    pub fn parent(self) -> HWND {
        match self {
            Layer::Raised { progman, .. } => progman,
            Layer::Classic { workerw } => workerw,
        }
    }

    pub fn is_alive(self) -> bool {
        unsafe {
            match self {
                Layer::Raised { progman, icons } => {
                    IsWindow(Some(progman)).as_bool() && IsWindow(Some(icons)).as_bool()
                }
                Layer::Classic { workerw } => IsWindow(Some(workerw)).as_bool(),
            }
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Layer::Raised { .. } => "Windows 11 24H2+ (слой внутри Progman)",
            Layer::Classic { .. } => "классический (WorkerW за значками)",
        }
    }
}

fn find_child(parent: Option<HWND>, after: Option<HWND>, class: PCWSTR) -> Option<HWND> {
    unsafe { FindWindowExW(parent, after, class, PCWSTR::null()) }
        .ok()
        .filter(|h| !h.is_invalid())
}

unsafe extern "system" fn find_icons_host(top: HWND, out: LPARAM) -> BOOL {
    if find_child(Some(top), None, w!("SHELLDLL_DefView")).is_some() {
        *(out.0 as *mut HWND) = top;
        return BOOL(0);
    }
    BOOL(1)
}

fn spawn_workerw(progman: HWND) {
    // (0xD, 1) — значение Lively: на 24H2 только оно создаёт слой.
    // (0, 0) — классический вариант для старых сборок.
    for (wp, lp) in [(0xD, 1), (0, 0)] {
        unsafe {
            SendMessageTimeoutW(
                progman,
                0x052C,
                WPARAM(wp),
                LPARAM(lp),
                SMTO_NORMAL | SMTO_BLOCK,
                1000,
                None,
            );
        }
    }
}

/// Находит слой для обоев. WorkerW появляется не сразу после старта
/// проводника, поэтому несколько попыток.
pub fn find_layer() -> Option<Layer> {
    for attempt in 0..10 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        let Some(progman) = find_child(None, None, w!("Progman")) else {
            continue;
        };
        spawn_workerw(progman);

        if let Some(icons) = find_child(Some(progman), None, w!("SHELLDLL_DefView")) {
            if find_child(Some(progman), None, w!("WorkerW")).is_some() {
                return Some(Layer::Raised { progman, icons });
            }
        }

        let mut host = HWND::default();
        unsafe {
            let _ = EnumWindows(
                Some(find_icons_host),
                LPARAM(&mut host as *mut HWND as isize),
            );
        }
        if !host.is_invalid() {
            if let Some(workerw) = find_child(None, Some(host), w!("WorkerW")) {
                return Some(Layer::Classic { workerw });
            }
        }

        // Значки внутри Progman, а WorkerW нет — всё равно можно встать
        // под значки, как на 24H2.
        if attempt == 9 {
            if let Some(icons) = find_child(Some(progman), None, w!("SHELLDLL_DefView")) {
                return Some(Layer::Raised { progman, icons });
            }
        }
    }
    None
}

#[derive(Clone, Debug)]
pub struct Monitor {
    pub device: String,
    pub rect: RECT,
    pub primary: bool,
}

impl Monitor {
    pub fn width(&self) -> i32 {
        self.rect.right - self.rect.left
    }
    pub fn height(&self) -> i32 {
        self.rect.bottom - self.rect.top
    }
}

unsafe extern "system" fn collect_monitor(m: HMONITOR, _: HDC, _: *mut RECT, out: LPARAM) -> BOOL {
    let list = &mut *(out.0 as *mut Vec<Monitor>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(m, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
        let len = info
            .szDevice
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(info.szDevice.len());
        list.push(Monitor {
            device: String::from_utf16_lossy(&info.szDevice[..len]),
            rect: info.monitorInfo.rcMonitor,
            primary: info.monitorInfo.dwFlags & 1 != 0, // MONITORINFOF_PRIMARY
        });
    }
    BOOL(1)
}

/// Мониторы: основной первым, остальные слева направо.
pub fn monitors() -> Vec<Monitor> {
    let mut list: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(&mut list as *mut Vec<Monitor> as isize),
        );
    }
    list.sort_by_key(|m| (!m.primary, m.rect.left, m.rect.top));
    list
}

unsafe extern "system" fn host_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => {
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            FillRect(HDC(wp.0 as _), &rc, HBRUSH(GetStockObject(BLACK_BRUSH).0));
            LRESULT(1)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            FillRect(hdc, &ps.rcPaint, HBRUSH(GetStockObject(BLACK_BRUSH).0));
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

const HOST_CLASS: PCWSTR = w!("ZhivyeOboiHost");

fn register_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(host_proc),
            hInstance: instance.into(),
            lpszClassName: HOST_CLASS,
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);
    });
}

/// Создаёт окно обоев на месте монитора и вешает его в слой.
pub fn create_host(layer: Layer, monitor: &Monitor) -> Option<HWND> {
    register_class();
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            HOST_CLASS,
            w!("Живые обои"),
            WS_POPUP,
            monitor.rect.left,
            monitor.rect.top,
            monitor.width(),
            monitor.height(),
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;
        // Полностью непрозрачное слоистое окно. Альфа ставится до SetParent,
        // иначе на 24H2 окно не попадает в композицию.
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
        attach(hwnd, layer, monitor);
        Some(hwnd)
    }
}

/// Вешает окно в слой и ставит на место монитора в координатах родителя.
pub fn attach(hwnd: HWND, layer: Layer, monitor: &Monitor) {
    unsafe {
        let parent = layer.parent();
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let style = (style & !(WS_POPUP.0 | WS_CAPTION.0 | WS_THICKFRAME.0)) | WS_CHILD.0;
        SetWindowLongPtrW(hwnd, GWL_STYLE, style as isize);
        let _ = SetParent(hwnd, Some(parent));

        let mut pts = [
            POINT {
                x: monitor.rect.left,
                y: monitor.rect.top,
            },
            POINT {
                x: monitor.rect.right,
                y: monitor.rect.bottom,
            },
        ];
        MapWindowPoints(None, Some(parent), &mut pts);
        let insert_after = match layer {
            Layer::Raised { icons, .. } => icons,
            Layer::Classic { .. } => HWND_TOP,
        };
        let _ = SetWindowPos(
            hwnd,
            Some(insert_after),
            pts[0].x,
            pts[0].y,
            pts[1].x - pts[0].x,
            pts[1].y - pts[0].y,
            SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_FRAMECHANGED,
        );
    }
}

pub fn host_alive(hwnd: HWND, layer: Layer) -> bool {
    unsafe { IsWindow(Some(hwnd)).as_bool() && GetParent(hwnd).ok() == Some(layer.parent()) }
}

pub fn destroy(hwnd: HWND) {
    unsafe {
        if IsWindow(Some(hwnd)).as_bool() {
            let _ = DestroyWindow(hwnd);
        }
    }
}

/// Перерисовывает обычные обои Windows — после закрытия программы в
/// WorkerW не остаётся последнего кадра.
pub fn refresh_system_wallpaper() {
    unsafe {
        let mut buf = [0u16; 1024];
        if SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buf.len() as u32,
            Some(buf.as_mut_ptr().cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
        {
            let _ = SystemParametersInfoW(
                SPI_SETDESKWALLPAPER,
                0,
                Some(buf.as_mut_ptr().cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
        }
    }
}

/// Окно на весь экран поверх рабочего стола (игра, фильм, презентация).
pub fn fullscreen_app_active(own: &[HWND]) -> bool {
    use windows::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE,
        QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    unsafe {
        if let Ok(state) = SHQueryUserNotificationState() {
            if state == QUNS_BUSY
                || state == QUNS_RUNNING_D3D_FULL_SCREEN
                || state == QUNS_PRESENTATION_MODE
            {
                return true;
            }
        }
        let fg = GetForegroundWindow();
        if fg.is_invalid() || own.contains(&fg) {
            return false;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(fg, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..n]);
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
            return false;
        }
        let mut rc = RECT::default();
        if GetWindowRect(fg, &mut rc).is_err() {
            return false;
        }
        monitors().iter().any(|m| {
            rc.left <= m.rect.left
                && rc.top <= m.rect.top
                && rc.right >= m.rect.right
                && rc.bottom >= m.rect.bottom
        })
    }
}
