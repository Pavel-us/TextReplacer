# Contributing / Участие в проекте

[English](#english) · [Русский](#русский)

## English

Thanks for helping out! Bug reports, ideas, translations and code are all welcome.

### Development setup
1. Install [Rust](https://rustup.rs), Node.js 18+ and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.
2. `npm install`
3. `npm run dev` — run the app; `npm run test` — translation check + engine tests.

### Project layout
| Path | Purpose |
|---|---|
| `src-tauri/src/engine.rs` | Search & replace engine (no Tauri dependency, unit-tested) |
| `src-tauri/src/main.rs` | Tauri commands |
| `src/` | Frontend (plain HTML/CSS/JS, no bundler) |
| `src/locales/` | Translations |
| `scripts/check-locales.mjs` | Translation validator (runs in CI) |

### Adding a language
1. Copy `src/locales/en.json` to `src/locales/<code>.json` (e.g. `de.json`, `pt-br.json`; use lowercase).
2. Set `_meta.code` (same as the file name) and `_meta.name` (the language name in that language, e.g. `Deutsch`).
3. Translate every value. Keep placeholders such as `{0}` / `{1}` and line breaks (`\n`) intact.
4. Add the code to `src/locales/index.json`.
5. Run `npm run check:locales` — it reports missing keys, unknown keys and broken placeholders.
6. Open a pull request.

Messages that come from the engine (`skip.*`, `err.*`) are translated through the same files.
If you add new UI text, add the key to **every** locale file (use the English text as a temporary value) and mention it in the PR.

Users can also add a language **without rebuilding**: see "Languages" in the README.

### Pull requests
- Keep changes focused; add or update unit tests for engine changes (`cargo test --manifest-path src-tauri/Cargo.toml`).
- Code comments and commit messages in English, please.

## Русский

Спасибо за помощь! Приветствуются отчёты об ошибках, идеи, переводы и код.

### Окружение разработки
1. Установите [Rust](https://rustup.rs), Node.js 18+ и [зависимости Tauri](https://v2.tauri.app/start/prerequisites/) для вашей ОС.
2. `npm install`
3. `npm run dev` — запуск приложения; `npm run test` — проверка переводов и тесты движка.

### Структура проекта
| Путь | Назначение |
|---|---|
| `src-tauri/src/engine.rs` | Движок поиска и замены (без зависимости от Tauri, покрыт тестами) |
| `src-tauri/src/main.rs` | Команды Tauri |
| `src/` | Интерфейс (чистые HTML/CSS/JS, без сборщика) |
| `src/locales/` | Переводы |
| `scripts/check-locales.mjs` | Проверка переводов (запускается в CI) |

### Как добавить язык
1. Скопируйте `src/locales/en.json` в `src/locales/<код>.json` (например `de.json`, `pt-br.json`; строчными буквами).
2. Укажите `_meta.code` (как имя файла) и `_meta.name` (название языка на самом языке, например `Deutsch`).
3. Переведите все значения. Не трогайте плейсхолдеры `{0}` / `{1}` и переводы строк (`\n`).
4. Добавьте код в `src/locales/index.json`.
5. Запустите `npm run check:locales` — скрипт покажет пропущенные и лишние ключи и сломанные плейсхолдеры.
6. Откройте pull request.

Сообщения движка (`skip.*`, `err.*`) переводятся через те же файлы.
Если вы добавляете новый текст в интерфейс, добавьте ключ в **каждый** файл перевода (временно можно английский текст) и упомяните об этом в PR.

Пользователь может добавить язык и **без пересборки**: см. раздел «Языки» в README.

### Pull request'ы
- Делайте небольшие сфокусированные изменения; для правок движка добавляйте тесты (`cargo test --manifest-path src-tauri/Cargo.toml`).
- Комментарии в коде и сообщения коммитов — на английском.
