# Changelog / История изменений

All notable changes are documented here. / Все заметные изменения описаны здесь.

## [1.4.2] — 2026-11-12

<!-- lang:en -->
### Fixed
- You can now copy a line together with the tabs or spaces at its end and replace it with the same text without them. Tabs at the start or end of a paragraph used to be invisible to the search, so a "Find" text containing a tab never matched in Word, LibreOffice and PowerPoint documents. Now they are matched (and removed) when the "Find" text contains a tab, and a regular expression such as `\t+$` can strip them. Without a tab in the "Find" text, trailing tabs are left untouched as before.

<!-- lang:ru -->
### Исправлено
- Теперь можно скопировать строку вместе с табуляцией или пробелами в конце и заменить её на тот же текст без них. Табуляции в начале и в конце абзаца раньше были невидимы для поиска, поэтому текст с табуляцией в поле «Найти» не находился в документах Word, LibreOffice и PowerPoint. Теперь такие табуляции находятся (и удаляются), если в поле «Найти» есть табуляция, а регулярное выражение вроде `\t+$` может их убрать. Если в тексте поиска табуляции нет, хвостовые табуляции, как и раньше, остаются нетронутыми.

## [1.4.1] — 2026-11-12

<!-- lang:en -->
### Fixed
- Multi-line find now ignores blanks around line breaks. Spaces, tabs and no-break spaces at the end or start of a line no longer prevent a match: documents often keep such invisible characters (for example a line ending in two tabs) and they are lost when the text is copied into the "Find" field. Blanks at the very start and end of the whole text are still matched exactly.

<!-- lang:ru -->
### Исправлено
- Многострочный поиск теперь игнорирует пробелы вокруг переносов строк. Пробелы, табуляции и неразрывные пробелы в конце или начале строки больше не мешают найти текст: в документах часто остаются такие невидимые символы (например, строка, заканчивающаяся двумя табуляциями), а при копировании текста в поле «Найти» они теряются. Пробелы в самом начале и в самом конце всего текста по-прежнему сравниваются точно.

## [1.4.0] — 2026-11-11

Multi-line search and replace across paragraph boundaries in office documents (Word, PowerPoint, LibreOffice, RTF). Table cells remain barriers.

Поиск и замена через границы абзацев в офисных документах (Word, PowerPoint, LibreOffice, RTF). Ячейки таблиц остаются барьером.

### Changed / Изменено
- Multi-line find now works across paragraph boundaries in .docx/.docm/.dotx/.dotm, .pptx/.pptm/.potx/.potm/.ppsx/.ppsm, .odt/.odp/.ods and .rtf: put several lines with line breaks into `Find`. Table cells remain barriers — a match cannot cross a cell boundary. Single-line finds still match within one paragraph/cell only.
- Многострочный поиск теперь работает через границы абзацев в .docx/.docm/.dotx/.dotm, .pptx/.pptm/.potx/.potm/.ppsx/.ppsm, .odt/.odp/.ods и .rtf — укажите в «Найти» несколько строк с переносами. Ячейки таблиц остаются барьером: совпадение не выходит за границу ячейки. Однострочные поиски по-прежнему работают внутри одного абзаца/ячейки.

## [1.3.0] — unreleased / не выпущено

<!-- lang:en -->
### Added
- Help window (the **?** button, top right): quick start, supported formats and ten ready-made regular-expression recipes that can be applied with one click.
- Automatic update check with a window that shows what is new. Windows, macOS and AppImage update in place; .deb / .rpm installs get a link to the downloads page. This is the first version with updates, so install it manually once.
- The app is listed in the "Office" category of Linux application menus.

### Changed
- In regular-expression mode, replacement text with line breaks now follows the file's line-ending style (LF / CRLF).
- Multi-line find now works across paragraph boundaries in .docx/.docm/.dotx/.dotm, .pptx/.pptm/.potx/.potm/.ppsx/.ppsm, .odt/.odp/.ods and .rtf: put several lines with line breaks into `Find`. Table cells remain barriers — a match cannot cross a cell boundary. Single-line finds still match within one paragraph/cell only.

### Removed
- Automatic conversion of old .doc / .xls / .ppt files: it lost fonts and tables and could alter the text. These files are now skipped with a hint — save them as .docx / .xlsx / .pptx in Word or LibreOffice first.

<!-- lang:ru -->
### Добавлено
- Окно справки (кнопка **?** справа вверху): быстрый старт, поддерживаемые форматы и десять готовых рецептов регулярных выражений, применяемых одним нажатием.
- Автоматическая проверка обновлений с окном «что нового». Windows, macOS и AppImage обновляются на месте; для установки из .deb / .rpm показывается ссылка на страницу загрузки. Это первая версия с обновлением, поэтому установите её вручную один раз.
- Приложение отображается в разделе «Офис» меню приложений Linux.

### Изменено
- В режиме регулярных выражений переводы строк в тексте замены теперь соответствуют стилю файла (LF / CRLF).
- Многострочный поиск теперь работает через границы абзацев в .docx/.docm/.dotx/.dotm, .pptx/.pptm/.potx/.potm/.ppsx/.ppsm, .odt/.odp/.ods и .rtf — укажите в «Найти» несколько строк с переносами. Ячейки таблиц остаются барьером: совпадение не выходит за границу ячейки. Однострочные поиски по-прежнему работают внутри одного абзаца/ячейки.

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
