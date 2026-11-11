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
- `.rtf` edited in place with formatting kept
- Built-in help (**?** button) with a quick start and ten ready-made regular-expression recipes
- Automatic update check that shows what is new before updating
- **Interface languages**: English and Russian, more can be added without rebuilding

## Supported formats

| Format | Extensions | What is processed |
|---|---|---|
| Plain text | any (selected by mask) | UTF-8 / UTF-16 / cp1251 |
| Word | docx, docm, dotx, dotm | body, tables, headers/footers, footnotes |
| Excel | xlsx, xlsm, xltx, xltm | string cell values (shared and inline strings) |
| PowerPoint | pptx, pptm, potx, potm, ppsx, ppsm | slide and notes text |
| LibreOffice / OpenOffice | odt, ods, odp, odg, ott, ots, otp, otg | `content.xml`, `styles.xml` (incl. headers/footers) |
| Legacy Office | doc, xls, ppt | not supported — save as docx / xlsx / pptx first (such files are skipped with a hint) |
| RTF | rtf | edited in place, formatting kept (Cyrillic and other code pages supported) |
| PDF *(experimental)* | pdf | Latin text in simple fonts only — see below |

In office formats the formatting is preserved even if a phrase is split into runs with different styles.
A search may span paragraphs: put several lines into `Find`. Table cells remain barriers — a match cannot cross a cell boundary.
Not touched: formulas, sheet names, comments, charts, document properties.

### Legacy formats (.doc, .xls, .ppt)

Old binary Word 97 / Excel 97 / PowerPoint 97 files are **skipped** (the results table says so). They cannot be edited safely in place,
and automatic conversion turned out to lose fonts and tables and even alter the text. Open such a file in Word or LibreOffice,
save it as `.docx` / `.xlsx` / `.pptx`, then process the new file.

### RTF

`.rtf` is edited **in place** by a built-in parser: only the text is replaced, all formatting control words stay.
Text split by bold / italic / colour changes is still found. A search may span paragraphs (put line breaks into `Find`), but not table cells.
Code pages (`\ansicpg`, per-font charsets: 1250–1258, 874, 866, KOI8) and `\uN` escapes are handled; replacement text is written as `\uN` escapes.

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

## Help and recipes

The **?** button in the top right corner opens the help window: a quick start, the list of supported formats and ten ready-made
regular-expression recipes (collapse spaces, reformat dates, strip HTML tags, change a domain …). **Use** fills in the fields
and switches regular expressions on — then adapt the recipe to your case.

## Updates

On startup the app looks for a newer version on GitHub Releases (this can be switched off in Help → About) and, if there is one,
shows a window with **what's new**. You decide: update now, later, or skip this version.
Windows, macOS and AppImage update themselves (the download is verified by a signature); a `.deb` / `.rpm` installation
cannot update itself, so the window offers the downloads page instead. Installing the new `.deb` over the old one is enough.

On Linux the `.deb` package puts the app into the **Office** category of the application menu.

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

## Releases and automatic updates

*For maintainers.* Updates are signed, so a key pair is needed once:

1. `npx tauri signer generate -w ~/.tauri/textreplacer.key` (an empty password is fine).
2. `node scripts/set-updater-key.mjs ~/.tauri/textreplacer.key.pub`, then commit `src-tauri/tauri.conf.json`.
3. On GitHub: *Settings → Secrets and variables → Actions → New repository secret*:
   `TAURI_SIGNING_PRIVATE_KEY` = the whole content of `~/.tauri/textreplacer.key`
   (and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if you set a password).
4. **Back up the private key.** If it is lost, installed copies can no longer be updated.

To publish a version: describe the changes in `CHANGELOG.md` under `## [x.y.z]` using the `<!-- lang:en -->` / `<!-- lang:ru -->`
markers (this text becomes the release notes and the "what's new" in the update window), raise the version in `package.json`,
`src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`, then push a tag: `git tag vX.Y.Z && git push origin vX.Y.Z`.
The workflow builds all platforms, signs the files, creates the release and uploads `latest.json`.
Local builds without signing: `npm run build:local`.

## Project layout

```text
src/                  frontend (HTML/CSS/JS), src/locales/ translations, src/recipes.json regex recipes
src-tauri/src/        engine.rs (search & replace engine) and main.rs (Tauri commands)
scripts/              check-locales.mjs — translation validator
.github/workflows/    CI and release builds
```

## Contributing

Issues and pull requests are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). Translations are especially appreciated.

## License

[MIT](LICENSE)
