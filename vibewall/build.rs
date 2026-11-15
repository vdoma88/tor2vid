// Ресурсы vibewall.exe: иконка, сведения о версии и манифест Windows —
// Common Controls 6 (окна выбора файлов), учёт DPI и совместимость с
// Windows 8+: без неё слоистые дочерние окна, на которых держатся обои в
// Windows 11 24H2, не работают. Целевая ОС проверяется в рантайме сборки,
// поэтому ресурсы встраиваются и при кросс-сборке из Linux (mingw windres).
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/vibewall.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = env!("CARGO_PKG_VERSION");
    let mut res = winresource::WindowsResource::new();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") && !cfg!(windows) {
        res.set_toolkit_path("/usr/bin")
            .set_windres_path("x86_64-w64-mingw32-windres")
            .set_ar_path("x86_64-w64-mingw32-ar");
    }
    res.set_icon("assets/icon.ico")
        .set_manifest_file("assets/vibewall.manifest")
        .set("ProductName", "Vibewall")
        .set("FileDescription", "Vibewall — живые обои для Windows")
        .set("CompanyName", "Vibewall")
        .set("LegalCopyright", "Vibewall. mpv — GPL, mpv.io")
        .set("OriginalFilename", "vibewall.exe")
        .set("InternalName", "vibewall")
        .set("ProductVersion", version)
        .set("FileVersion", version);
    res.compile().expect("не удалось встроить ресурсы Windows");
}
