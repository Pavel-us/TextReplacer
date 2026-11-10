# Changelog / История изменений

All notable changes are documented here. / Все заметные изменения описаны здесь.

## [1.2.0] — unreleased / не выпущено

### Changed / Изменено
- LibreOffice is no longer used. Legacy `.doc` / `.xls` / `.ppt` are converted by a built-in converter to `.docx` / `.xlsx` / `.pptx` and saved next to the original (the original is never modified).
  LibreOffice больше не используется. Старые `.doc` / `.xls` / `.ppt` конвертируются встроенным конвертером в `.docx` / `.xlsx` / `.pptx` и сохраняются рядом с оригиналом (оригинал не меняется).

### Added / Добавлено
- `.rtf` is edited in place by a built-in parser (code pages, `\uN`, formatting kept).
  `.rtf` правится на месте встроенным разборщиком (кодовые страницы, `\uN`, форматирование сохраняется).
- A parser failure on one file no longer crashes the application.
  Сбой разбора одного файла больше не роняет приложение.

## [1.1.0]

### Added / Добавлено
- Legacy `.doc` / `.xls` / `.ppt` / `.rtf` support through an installed LibreOffice (convert to a modern format, or edit and save back in the same format).
  Поддержка старых `.doc` / `.xls` / `.ppt` / `.rtf` через установленный LibreOffice (конвертация в современный формат или правка с сохранением в том же формате).

## [1.0.0] — 2026-11-09

### Added / Добавлено
- Desktop app for Windows, Linux and macOS (Tauri 2 + Rust engine).
  Десктопное приложение для Windows, Linux и macOS (Tauri 2 + движок на Rust).
- Batch find & replace with masks, subfolders, regex, whitespace-insensitive mode, `.bak` backups, dry-run preview.
  Пакетная замена с масками, подпапками, regex, режимом «пробелы не важны», `.bak` и предпросмотром.
- Formats: plain text (UTF-8/UTF-16/cp1251), Word, Excel, PowerPoint, LibreOffice/OpenOffice, experimental PDF.
  Форматы: текст (UTF-8/UTF-16/cp1251), Word, Excel, PowerPoint, LibreOffice/OpenOffice, экспериментально PDF.
- Interface in English and Russian; extra languages via JSON files, no rebuild needed.
  Интерфейс на английском и русском; дополнительные языки через JSON-файлы без пересборки.
- Drag-and-drop of folders, dark/light/system theme, remembered settings.
  Перетаскивание папок, тёмная/светлая/системная тема, запоминание настроек.
