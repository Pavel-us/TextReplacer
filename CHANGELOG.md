# Changelog / История изменений

All notable changes are documented here. / Все заметные изменения описаны здесь.

## [1.3.0] — unreleased / не выпущено

<!-- lang:en -->
### Added
- Help window (the **?** button, top right): quick start, supported formats and ten ready-made regular-expression recipes that can be applied with one click.
- Automatic update check with a window that shows what is new. Windows, macOS and AppImage update in place; .deb / .rpm installs get a link to the downloads page. This is the first version with updates, so install it manually once.
- The app is listed in the "Office" category of Linux application menus.

### Changed
- In regular-expression mode, replacement text with line breaks now follows the file's line-ending style (LF / CRLF).

### Removed
- Automatic conversion of old .doc / .xls / .ppt files: it lost fonts and tables and could alter the text. These files are now skipped with a hint — save them as .docx / .xlsx / .pptx in Word or LibreOffice first.

<!-- lang:ru -->
### Добавлено
- Окно справки (кнопка **?** справа вверху): быстрый старт, поддерживаемые форматы и десять готовых рецептов регулярных выражений, применяемых одним нажатием.
- Автоматическая проверка обновлений с окном «что нового». Windows, macOS и AppImage обновляются на месте; для установки из .deb / .rpm показывается ссылка на страницу загрузки. Это первая версия с обновлением, поэтому установите её вручную один раз.
- Приложение отображается в разделе «Офис» меню приложений Linux.

### Изменено
- В режиме регулярных выражений переводы строк в тексте замены теперь соответствуют стилю файла (LF / CRLF).

### Удалено
- Автоматическая конвертация старых файлов .doc / .xls / .ppt: она теряла шрифты и таблицы и могла искажать текст. Такие файлы теперь пропускаются с подсказкой — сначала сохраните их как .docx / .xlsx / .pptx в Word или LibreOffice.

## [1.2.0] — unreleased / не выпущено

### Changed / Изменено
- LibreOffice is no longer used. Legacy `.doc` / `.xls` / `.ppt` are converted by a built-in converter to `.docx` / `.xlsx` / `.pptx` and saved next to the original (the original is never modified).
  LibreOffice больше не используется. Старые `.doc` / `.xls` / `.ppt` конвертируются встроенным конвертером в `.docx` / `.xlsx` / `.pptx` и сохраняются рядом с оригиналом (оригинал не меняется).

### Changed / Изменено (menu)
- The app is listed in the "Office" category of Linux application menus.
  Приложение отображается в разделе «Офис» меню приложений Linux.

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
