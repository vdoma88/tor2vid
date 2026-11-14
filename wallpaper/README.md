# Живые обои

Приложение для Windows 10/11 на Rust: видео, GIF, картинки и звук на
рабочем столе, за значками. Своя библиотека, загрузка своих файлов и
поиск свободных роликов на Викискладе.

## Что умеет

- Видео (mp4, mkv, webm, mov…), анимация (GIF), картинки — на каждый
  монитор своё или одно на все.
- Звук: звук самого видео (с одного монитора, чтобы не было эха),
  спектр для звуковых файлов и фоновая музыка по кругу.
- Библиотека: «Добавить файлы» или перетаскивание в окно; превью кадра.
- Открытые ресурсы: поиск по Wikimedia Commons (только свободные
  лицензии), скачивание в библиотеку с автором и лицензией.
- Пауза, когда игра или фильм открыты на весь экран; трей; автозапуск;
  восстановление после перезапуска проводника и смены мониторов.

## Как устроено

| Файл | Что делает |
|---|---|
| `src/desktop.rs` | Окно за значками: `0x052C` в Progman, две раскладки — классическая (WorkerW) и Windows 11 24H2+ (слоистое дочернее окно Progman под `SHELLDLL_DefView`) |
| `src/mpv.rs` | libmpv, подгружается при запуске (`libmpv-2.dll`), опции обоев и музыки |
| `src/engine.rs` | Окно и mpv на каждый монитор, музыка, пауза, слежение за проводником |
| `src/library.rs` | Библиотека и настройки в `%LOCALAPPDATA%\zhivye-oboi` |
| `src/online.rs` | API Викисклада, скачивание с прогрессом |
| `src/thumbs.rs` | Превью через mpv `vo=image` |
| `src/ui.rs`, `src/theme.rs` | Панель на egui в стиле из `DESIGN.md` |

Подсмотрено в открытых проектах: [Lively](https://github.com/rocksdanister/lively)
(слой WorkerW, правки под 24H2, mpv), [weebp](https://github.com/Francesco149/weebp)
(`mpv --wid`), [BackScene](https://github.com/probablyxs/backscene).

## Сборка

```
cd wallpaper
cargo build --release
```

Рядом с `zhivye-oboi.exe` нужен `libmpv-2.dll` из `mpv-dev-x86_64-*.7z`
([shinchiro/mpv-winbuild-cmake](https://github.com/shinchiro/mpv-winbuild-cmake/releases)).
Без него программа запускается и пишет, чего не хватает.

Из Linux: `cargo build --release --target x86_64-pc-windows-gnu`
(нужен `gcc-mingw-w64-x86-64`). `cargo test` проверяет переносимую часть;
если установлен libmpv, тесты гоняют настоящий mpv.

Архив собирает `.github/workflows/wallpaper.yml` при изменениях в
`wallpaper/`, релиз — `oboi-v<version из Cargo.toml>`.

Диагностика у пользователя: `zhivye-oboi.exe --diagnose отчёт.txt`.

## Лицензии

mpv — GPL (сборка shinchiro), исходники на mpv.io. Файлы с Викисклада —
у каждого своя свободная лицензия, она сохраняется в библиотеке.
