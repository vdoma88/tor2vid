// Манифест Windows: Common Controls 6 (окна выбора файлов), учёт DPI и
// совместимость с Windows 8+ — без неё слоистые дочерние окна, на которых
// держатся обои в Windows 11 24H2, не работают.
fn main() {
    #[cfg(windows)]
    {
        use embed_manifest::manifest::{DpiAwareness, SupportedOS};
        use embed_manifest::{embed_manifest, new_manifest};
        if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
            embed_manifest(
                new_manifest("ZhivyeOboi")
                    .dpi_awareness(DpiAwareness::PerMonitorV2)
                    .supported_os(SupportedOS::Windows7..=SupportedOS::Windows10),
            )
            .expect("не удалось встроить манифест");
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
