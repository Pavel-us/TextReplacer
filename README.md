# Text Replacer

**English** · [Русский](README.ru.md)

[![CI](https://github.com/Pavel-us/TextReplacer/actions/workflows/ci.yml/badge.svg)](https://github.com/Pavel-us/TextReplacer/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A fast cross-platform desktop tool for **batch find & replace in files**. Works on Windows, Linux and macOS.
A modern alternative to classic "Text Replacer" utilities: the engine is written in Rust (files are processed in parallel),
the interface is built with [Tauri](https://tauri.app).

## Features

- Recursive folder walk, file name masks (`*.html;*.txt`), `.git` is skipped
- Multi-line search and replacement; exact matching (edge spaces are preserved)
- "Whitespace and line breaks don't matter" mode, case-sensitive / insensitive search
- Regular expressions with capture groups (`$1`, `$2` in the replacement)
- **Batch replacement**: a list of rules executed in order, or a rules file
- **Preview** (dry run), `.bak` backups, atomic writes, file size limit
- Auto-detects encodings (UTF-8, UTF-16, cp1251) and keeps line-ending style (LF / CRLF)
- Drag-and-drop a folder or file into the window
- Dark / light / system theme, remembered settings
- Legacy `.doc` / `.xls` / `.ppt` / `.rtf` through LibreOffice (optional)
- **Interface languages**: English and Russian, more can be added without rebuilding

## Supported formats

| Format | Extensions | What is processed |
|---|---|---|
| Plain text | any (selected by mask) | UTF-8 / UTF-16 / cp1251 |
| Word | docx, docm, dotx, dotm | body, tables, headers/footers, footnotes |
| Excel | xlsx, xlsm, xltx, xltm | string cell values (shared and inline strings) |
| PowerPoint | pptx, pptm, potx, potm, ppsx, ppsm | slide and notes text |
| LibreOffice / OpenOffice | odt, ods, odp, odg, ott, ots, otp, otg | `content.xml`, `styles.xml` (incl. headers/footers) |
| Legacy Office | doc, xls, ppt, rtf | via [LibreOffice](https://www.libreoffice.org) (must be installed) — see below |
| PDF *(experimental)* | pdf | Latin text in simple fonts only — see below |

In office formats the formatting is preserved even if a phrase is split into runs with different styles.
A match must lie within one paragraph (in spreadsheets: within one cell).
Not touched: formulas, sheet names, comments, charts, document properties.

### Legacy formats (.doc, .xls, .ppt, .rtf)

Binary Word 97 / Excel 97 / PowerPoint 97 files cannot be edited safely in place, so the app uses a
locally installed **LibreOffice** (detected automatically, or set its path in the sidebar):

1. the file is converted to `.docx` / `.xlsx` / `.pptx` in a temporary folder,
2. the replacement is done there (formatting and fonts are preserved),
3. the result is saved according to the chosen mode:
   - **Convert and save as .docx/.xlsx/.pptx** — a new file is created next to the original, the original is untouched;
   - **Edit and save in the same format** — the result is converted back and overwrites the original
     (enable `.bak` backups; LibreOffice re-encodes the file, so rare layout differences are possible).

It is off by default and slower (about 2 seconds per file, files are converted one at a time).
`.rtf` also works this way, which makes Cyrillic text in RTF searchable.

### PDF limitations

A PDF stores drawing instructions for font glyphs, not text, so editing is inherently limited:

- Only Latin text in simple fonts is found. Cyrillic and text in Identity-H fonts (most modern PDFs) is not.
- Spaces between words are often positioning offsets, not characters — search for single words rather than phrases.
- Embedded fonts usually contain only the glyphs already used: new characters may render blank. Layout is not re-flowed.
- Encrypted or damaged PDFs are skipped.

For serious PDF editing, convert to docx, edit, and export back. The PDF module can be disabled at build time:
`cargo build --no-default-features`.

## Download

Ready-made builds (Windows `.msi`/`.exe`, Linux `.deb`/`.AppImage`, macOS `.dmg`) are published on the
[Releases](https://github.com/Pavel-us/TextReplacer/releases) page.

## Usage

1. Choose a folder (or drop it into the window) and, optionally, a file name mask.
2. Type the text to find and the replacement. Click **＋ Add to list** to build a batch of rules,
   or load rules from a file.
3. Click **Preview** to see what would change, then **Replace**.
4. Double-click a row in the results to open the file's folder.

**Rules file format** — one rule per line, `find<TAB>replace`; `\n` and `\t` are allowed; lines starting with `#` are comments:

```text
# old brand -> new brand
Acme Corp	Globex Inc
http://old.example.com	https://new.example.com
```

**Regular expressions** use the Rust [`regex`](https://docs.rs/regex) syntax: no look-around or backreferences inside the pattern,
but no catastrophic backtracking either. Use `$1`, `$2` or `${name}` in the replacement; `$$` is a literal dollar.
In normal (non-regex) mode `$` is always a literal character.

## Languages

The interface language is selected in the sidebar. The app picks your system language on first start.

**Add a language without rebuilding** (for users):

1. Click the 📂 button next to the language selector — it opens the translations folder and creates `_template.json` there:
   - Linux: `~/.config/dev.textreplacer.app/locales/`
   - macOS: `~/Library/Application Support/dev.textreplacer.app/locales/`
   - Windows: `%APPDATA%\dev.textreplacer.app\locales\`
2. Copy `_template.json` to e.g. `de.json`, set `_meta.code` (`de`) and `_meta.name` (`Deutsch`), translate the values.
   Keep placeholders like `{0}` unchanged. Files starting with `_` are ignored.
3. Switch back to the app — the new language appears in the list. Missing keys fall back to English.

**Add a language to the project** (for contributors): see [CONTRIBUTING.md](CONTRIBUTING.md#adding-a-language).
`npm run check:locales` validates your file.

## Build from source

Requirements: [Rust](https://rustup.rs), Node.js 18+, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)
(Windows: MSVC Build Tools; Debian/Ubuntu: `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`).

```bash
npm install
npm run dev        # development window
npm run build      # installers and binary in src-tauri/target/release/
npm run test       # translations check + engine unit tests
```

Cross-compilation is not supported: build on each target OS, or let GitHub Actions do it
(**Actions → Build → Run workflow**, or push a `v*` tag to get a Release with archives).

## Project layout

```text
src/                  frontend (HTML/CSS/JS) and src/locales/ translations
src-tauri/src/        engine.rs (search & replace engine) and main.rs (Tauri commands)
scripts/              check-locales.mjs — translation validator
.github/workflows/    CI and release builds
```

## Contributing

Issues and pull requests are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). Translations are especially appreciated.

## License

[MIT](LICENSE)
