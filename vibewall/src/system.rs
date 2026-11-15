//! Мелочи Windows: автозапуск, открытие ссылок и папок, окно панели.

use std::sync::OnceLock;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_SET_VALUE, REG_SZ,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, MessageBoxW, SendMessageTimeoutW, SetForegroundWindow, ShowWindow, MB_ICONERROR,
    MB_OK, SMTO_ABORTIFHUNG, SW_HIDE, SW_RESTORE, SW_SHOWNORMAL, WM_APP,
};

/// Сообщение управляющему окну: «покажи панель» (от второго запуска).
pub const WM_SHOW_PANEL: u32 = WM_APP + 1;
pub const CONTROLLER_CLASS: PCWSTR = w!("VibewallController");

static PANEL: OnceLock<Panel> = OnceLock::new();

pub fn set_panel(p: Panel) {
    let _ = PANEL.set(p);
}

pub fn show_panel() {
    if let Some(p) = PANEL.get() {
        p.show();
    }
}

/// Программа уже запущена? Тогда просим её показать окно. Если та копия
/// не отвечает (зависла), запускаемся как обычно.
pub fn signal_running_instance() -> bool {
    unsafe {
        match FindWindowW(CONTROLLER_CLASS, PCWSTR::null()) {
            Ok(h) if !h.is_invalid() => {
                let mut result = 0usize;
                SendMessageTimeoutW(
                    h,
                    WM_SHOW_PANEL,
                    WPARAM(0),
                    LPARAM(0),
                    SMTO_ABORTIFHUNG,
                    3000,
                    Some(&mut result),
                )
                .0 != 0
            }
            _ => false,
        }
    }
}

/// Окно с ошибкой — у программы нет консоли, иначе сбой был бы немым.
pub fn error_box(text: &str) {
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from("Vibewall"),
            MB_OK | MB_ICONERROR,
        );
    }
}

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_NAME: PCWSTR = w!("Vibewall");
/// Имя автозапуска до переименования — убираем при включении нового.
const OLD_RUN_NAME: PCWSTR = w!("ZhivyeOboi");

/// Запуск вместе с Windows: запись в HKCU\…\Run, программа стартует
/// свёрнутой в трей и восстанавливает обои.
pub fn set_autostart(on: bool) -> Result<(), String> {
    unsafe {
        let mut key = HKEY::default();
        RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, KEY_SET_VALUE, &mut key)
            .ok()
            .map_err(|e| e.to_string())?;
        // Запись от версии до переименования больше не нужна.
        let _ = RegDeleteValueW(key, OLD_RUN_NAME);
        let res = if on {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let value = format!("\"{}\" --tray", exe.display());
            let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes = std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2);
            RegSetValueExW(key, RUN_NAME, None, REG_SZ, Some(bytes)).ok()
        } else {
            let r = RegDeleteValueW(key, RUN_NAME);
            if r.0 == 2 {
                Ok(())
            } else {
                r.ok()
            } // ERROR_FILE_NOT_FOUND — и так нет
        };
        let _ = RegCloseKey(key);
        res.map_err(|e| e.to_string())
    }
}

pub fn open(target: &str) {
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(target),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Окно панели управления. Показываем и прячем его сами, через Win32:
/// пока окно скрыто, eframe не крутит кадры и команды ему не доходят.
#[derive(Clone, Copy)]
pub struct Panel(pub HWND);

unsafe impl Send for Panel {}
unsafe impl Sync for Panel {}

impl Panel {
    pub fn show(self) {
        unsafe {
            let _ = ShowWindow(self.0, SW_RESTORE);
            let _ = SetForegroundWindow(self.0);
        }
    }

    pub fn hide(self) {
        unsafe {
            let _ = ShowWindow(self.0, SW_HIDE);
        }
    }
}
