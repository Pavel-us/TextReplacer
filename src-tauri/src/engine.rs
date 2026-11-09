//! Search & replace engine. Independent of Tauri, so it is tested on its own (`cargo test`).
//!
//! User-facing messages are returned as codes ("skip.binary", "err.empty_find\u{1f}3" …):
//! the part after \u{1f} is an argument. The frontend translates them via src/locales/*.json.

use encoding_rs::WINDOWS_1251;
use rayon::prelude::*;
use regex::{Captures, Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use walkdir::WalkDir;
use zip::{write::SimpleFileOptions, CompressionMethod};

type Res<T> = Result<T, Box<dyn Error>>;

// ───────────── types exchanged with the UI ─────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Rule {
    pub find: String,
    pub repl: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub folder: String,
    pub mask: String,
    pub rules: Vec<Rule>,
    pub recursive: bool,
    pub case_sensitive: bool,
    pub ignore_ws: bool,
    pub regex: bool,
    pub backup: bool,
    pub binary: bool,
    pub dry: bool,
    /// 0 = no limit
    pub max_size_mb: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileEvent {
    pub path: String,
    /// ok | skipped | error
    pub status: String,
    pub count: usize,
    pub info: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Stats {
    pub files: usize,
    pub changed: usize,
    pub replaced: usize,
    pub skipped: usize,
    pub errors: usize,
}

// ───────────── rules ─────────────

pub struct Compiled {
    re: Regex,
    repl: String,
    is_regex: bool,
}

impl Compiled {
    fn expand(&self, caps: &Captures, nl: &str) -> String {
        if self.is_regex {
            let mut s = String::new();
            caps.expand(&self.repl, &mut s);
            s
        } else {
            self.repl.replace('\n', nl)
        }
    }
}

pub fn compile(rules: &[Rule], o: &Request) -> Result<Vec<Compiled>, String> {
    if rules.is_empty() {
        return Err("err.no_rules".into());
    }
    let mut out = Vec::with_capacity(rules.len());
    for (i, r) in rules.iter().enumerate() {
        let n = i + 1;
        if r.find.is_empty() {
            return Err(format!("err.empty_find\u{1f}{n}"));
        }
        let pattern = if o.regex {
            r.find.clone()
        } else if o.ignore_ws {
            // any whitespace / line breaks between words are treated as equal
            let toks: Vec<String> = r.find.split_whitespace().map(regex::escape).collect();
            if toks.is_empty() {
                return Err(format!("err.only_ws\u{1f}{n}"));
            }
            toks.join(r"\s+")
        } else {
            // exact match, edge spaces are kept; \n also matches \r\n
            r.find
                .split('\n')
                .map(|l| regex::escape(l.trim_end_matches('\r')))
                .collect::<Vec<_>>()
                .join(r"\r?\n")
        };
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(!o.case_sensitive)
            .multi_line(true)
            .build()
            .map_err(|e| format!("err.regex\u{1f}{n}\u{1f}{e}"))?;
        out.push(Compiled {
            re,
            repl: if o.regex { r.repl.clone() } else { r.repl.replace("\r\n", "\n") },
            is_regex: o.regex,
        });
    }
    Ok(out)
}

pub fn apply_rules(text: &str, rules: &[Compiled], nl: &str) -> (String, usize) {
    let mut cur = text.to_string();
    let mut total = 0usize;
    for c in rules {
        let mut n = 0usize;
        let new = c
            .re
            .replace_all(&cur, |caps: &Captures| {
                n += 1;
                c.expand(caps, nl)
            })
            .into_owned();
        if n > 0 {
            cur = new;
            total += n;
        }
    }
    (cur, total)
}

// ───────────── encodings ─────────────

enum Enc {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Cp1251,
}

fn decode16(b: &[u8], le: bool) -> Option<String> {
    if b.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = b
        .chunks_exact(2)
        .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
        .collect();
    String::from_utf16(&units).ok()
}

fn decode(data: &[u8]) -> (String, Enc) {
    if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
        if let Ok(s) = std::str::from_utf8(&data[3..]) {
            return (s.to_string(), Enc::Utf8Bom);
        }
    }
    if data.starts_with(&[0xFF, 0xFE]) {
        if let Some(s) = decode16(&data[2..], true) {
            return (s, Enc::Utf16Le);
        }
    }
    if data.starts_with(&[0xFE, 0xFF]) {
        if let Some(s) = decode16(&data[2..], false) {
            return (s, Enc::Utf16Be);
        }
    }
    if let Ok(s) = std::str::from_utf8(data) {
        return (s.to_string(), Enc::Utf8);
    }
    let (s, _) = WINDOWS_1251.decode_without_bom_handling(data);
    (s.into_owned(), Enc::Cp1251)
}

fn encode(text: &str, enc: &Enc) -> Result<Vec<u8>, String> {
    match enc {
        Enc::Utf8 => Ok(text.as_bytes().to_vec()),
        Enc::Utf8Bom => {
            let mut v = vec![0xEF, 0xBB, 0xBF];
            v.extend_from_slice(text.as_bytes());
            Ok(v)
        }
        Enc::Utf16Le => {
            let mut v = vec![0xFF, 0xFE];
            for u in text.encode_utf16() {
                v.extend_from_slice(&u.to_le_bytes());
            }
            Ok(v)
        }
        Enc::Utf16Be => {
            let mut v = vec![0xFE, 0xFF];
            for u in text.encode_utf16() {
                v.extend_from_slice(&u.to_be_bytes());
            }
            Ok(v)
        }
        Enc::Cp1251 => {
            let (b, _, bad) = WINDOWS_1251.encode(text);
            if bad {
                Err("err.cp1251".into())
            } else {
                Ok(b.into_owned())
            }
        }
    }
}

// ───────────── writing files ─────────────

fn with_suffix(p: &Path, s: &str) -> PathBuf {
    let mut os = p.as_os_str().to_owned();
    os.push(s);
    PathBuf::from(os)
}

fn finish_write(path: &Path, tmp: &Path, meta: &fs::Metadata, backup: bool) -> Res<()> {
    if backup {
        fs::copy(path, with_suffix(path, ".bak"))?;
    }
    let _ = fs::set_permissions(tmp, meta.permissions());
    if let Err(e) = fs::rename(tmp, path) {
        let _ = fs::remove_file(tmp);
        return Err(e.into());
    }
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8], meta: &fs::Metadata, backup: bool) -> Res<()> {
    let tmp = with_suffix(path, ".tmp~");
    fs::write(&tmp, bytes)?;
    finish_write(path, &tmp, meta, backup)
}

// ───────────── plain text files ─────────────

enum Outcome {
    Unchanged,
    Changed(usize),
    Skipped(String),
}

fn process_text(path: &Path, meta: &fs::Metadata, rules: &[Compiled], o: &Request) -> Res<Outcome> {
    let data = fs::read(path)?;
    if data.is_empty() {
        return Ok(Outcome::Unchanged);
    }
    let utf16_bom = data.starts_with(&[0xFF, 0xFE]) || data.starts_with(&[0xFE, 0xFF]);
    if !o.binary && !utf16_bom && data[..data.len().min(8192)].contains(&0) {
        return Ok(Outcome::Skipped("skip.binary".into()));
    }
    let (text, enc) = decode(&data);
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count();
    let nl = if crlf > 0 && crlf * 2 >= lf { "\r\n" } else { "\n" };
    let (new, n) = apply_rules(&text, rules, nl);
    if n == 0 {
        return Ok(Outcome::Unchanged);
    }
    if new != text && !o.dry {
        let bytes = encode(&new, &enc)?;
        write_atomic(path, &bytes, meta, o.backup)?;
    }
    Ok(Outcome::Changed(n))
}

// ───────────── zipped documents: OOXML (Word/Excel/PowerPoint) and ODF (LibreOffice/OpenOffice) ─────────────

/// Compiles a regex once per call site.
macro_rules! re {
    ($pat:expr) => {{
        static R: OnceLock<Regex> = OnceLock::new();
        R.get_or_init(|| Regex::new($pat).unwrap())
    }};
}

// Opening tag that is NOT self-closing: `<w:p a="1"/>` must not glue neighbouring paragraphs together.
const W_P: &str = r"(?s)<w:p(?:\s(?:[^>/]|/[^>])*)?>.*?</w:p>";
const W_T: &str = r"(?s)(<w:t(?:\s(?:[^>/]|/[^>])*)?>)(.*?)(</w:t>)";
const A_P: &str = r"(?s)<a:p(?:\s(?:[^>/]|/[^>])*)?>.*?</a:p>";
const A_T: &str = r"(?s)(<a:t(?:\s(?:[^>/]|/[^>])*)?>)(.*?)(</a:t>)";
const X_SI: &str = r"(?s)<si>.*?</si>";
const X_IS: &str = r"(?s)<is>.*?</is>";
const X_T: &str = r"(?s)(<t(?:\s(?:[^>/]|/[^>])*)?>)(.*?)(</t>)";
const O_P: &str = r"(?s)<text:p(?:\s(?:[^>/]|/[^>])*)?>.*?</text:p>";
const O_H: &str = r"(?s)<text:h(?:\s(?:[^>/]|/[^>])*)?>.*?</text:h>";
const TAG: &str = r"<[^>]*>";

#[derive(Clone, Copy)]
enum Kind {
    Word,
    Excel,
    Slides,
    Odf,
}

fn doc_kind(path: &Path) -> Option<Kind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "docx" | "docm" | "dotx" | "dotm" => Kind::Word,
        "xlsx" | "xlsm" | "xltx" | "xltm" => Kind::Excel,
        "pptx" | "pptm" | "potx" | "potm" | "ppsx" | "ppsm" => Kind::Slides,
        "odt" | "ott" | "ods" | "ots" | "odp" | "otp" | "odg" | "otg" => Kind::Odf,
        _ => return None,
    })
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn xml_unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        if let Some(j) = after.find(';') {
            let ent = &after[1..j];
            let rep: Option<String> = match ent {
                "amp" => Some("&".into()),
                "lt" => Some("<".into()),
                "gt" => Some(">".into()),
                "quot" => Some("\"".into()),
                "apos" => Some("'".into()),
                _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                    u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32).map(|c| c.to_string())
                }
                _ if ent.starts_with('#') => {
                    ent[1..].parse::<u32>().ok().and_then(char::from_u32).map(|c| c.to_string())
                }
                _ => None,
            };
            if let Some(r) = rep {
                out.push_str(&r);
                rest = &after[j + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &after[1..];
    }
    out.push_str(rest);
    out
}

fn push_range(full: &str, bounds: &[(usize, usize)], out: &mut [String], a: usize, b: usize) {
    for (i, &(s, e)) in bounds.iter().enumerate() {
        let lo = a.max(s);
        let hi = b.min(e);
        if lo < hi {
            out[i].push_str(&full[lo..hi]);
        }
    }
}

/// Replaces in text glued together from pieces (runs), keeping the formatting boundaries.
fn replace_in_runs(runs: &[String], c: &Compiled) -> (Vec<String>, usize) {
    let full: String = runs.concat();
    let caps: Vec<Captures> = c
        .re
        .captures_iter(&full)
        .filter(|m| !m[0].is_empty())
        .collect();
    if caps.is_empty() {
        return (runs.to_vec(), 0);
    }
    let mut bounds = Vec::with_capacity(runs.len());
    let mut pos = 0;
    for r in runs {
        bounds.push((pos, pos + r.len()));
        pos += r.len();
    }
    let mut out = vec![String::new(); runs.len()];
    let mut cur = 0;
    for m in &caps {
        let w = m.get(0).unwrap();
        push_range(&full, &bounds, &mut out, cur, w.start());
        let owner = bounds
            .iter()
            .position(|&(s, e)| w.start() >= s && w.start() < e)
            .unwrap_or(0);
        out[owner].push_str(&c.expand(m, " "));
        cur = w.end();
    }
    push_range(&full, &bounds, &mut out, cur, full.len());
    (out, caps.len())
}

fn apply_to_runs(runs: &mut Vec<String>, rules: &[Compiled]) -> usize {
    let mut total = 0usize;
    for c in rules {
        let (r, n) = replace_in_runs(runs, c);
        if n > 0 {
            *runs = r;
            total += n;
        }
    }
    total
}

/// "Paragraph" units whose text lives in run tags (w:t, a:t, t).
fn replace_units(xml: &str, unit: &Regex, run: &Regex, preserve: bool, rules: &[Compiled]) -> (String, usize) {
    let mut count = 0usize;
    let out = unit
        .replace_all(xml, |pm: &Captures| {
            let p = &pm[0];
            let ts: Vec<Captures> = run.captures_iter(p).collect();
            if ts.is_empty() {
                return p.to_string();
            }
            let mut runs: Vec<String> = ts.iter().map(|t| xml_unescape(&t[2])).collect();
            let total = apply_to_runs(&mut runs, rules);
            if total == 0 {
                return p.to_string();
            }
            count += total;
            let mut idx = 0usize;
            run.replace_all(p, |tm: &Captures| {
                let mut tag = tm[1].to_string();
                if preserve && !tag.contains("xml:space") {
                    tag.pop();
                    tag.push_str(" xml:space=\"preserve\">");
                }
                let s = format!("{}{}{}", tag, xml_escape(&runs[idx]), &tm[3]);
                idx += 1;
                s
            })
            .into_owned()
        })
        .into_owned();
    (out, count)
}

/// ODF: text sits between tags inside text:p / text:h (with nested text:span etc.).
fn odf_units(xml: &str, unit: &Regex, rules: &[Compiled]) -> (String, usize) {
    let mut count = 0usize;
    let out = unit
        .replace_all(xml, |pm: &Captures| {
            let u = &pm[0];
            let mut segs: Vec<(usize, usize)> = Vec::new();
            let mut last = 0usize;
            for m in re!(TAG).find_iter(u) {
                if m.start() > last {
                    segs.push((last, m.start()));
                }
                last = m.end();
            }
            if last < u.len() {
                segs.push((last, u.len()));
            }
            if segs.is_empty() {
                return u.to_string();
            }
            let mut runs: Vec<String> = segs.iter().map(|&(a, b)| xml_unescape(&u[a..b])).collect();
            let total = apply_to_runs(&mut runs, rules);
            if total == 0 {
                return u.to_string();
            }
            count += total;
            let mut out = String::with_capacity(u.len());
            let mut cur = 0usize;
            for (i, &(a, b)) in segs.iter().enumerate() {
                out.push_str(&u[cur..a]);
                out.push_str(&xml_escape(&runs[i]));
                cur = b;
            }
            out.push_str(&u[cur..]);
            out
        })
        .into_owned();
    (out, count)
}

fn is_word_part(name: &str) -> bool {
    if !name.starts_with("word/") || !name.ends_with(".xml") || name[5..].contains('/') {
        return false;
    }
    let stem = &name[5..name.len() - 4];
    stem == "document" || stem == "footnotes" || stem == "endnotes"
        || stem.starts_with("header") || stem.starts_with("footer")
}

fn is_slide_part(name: &str) -> bool {
    name.ends_with(".xml")
        && ((name.starts_with("ppt/slides/slide") && !name["ppt/slides/".len()..].contains('/'))
            || (name.starts_with("ppt/notesSlides/notesSlide") && !name["ppt/notesSlides/".len()..].contains('/')))
}

fn process_part(kind: Kind, name: &str, xml: &str, rules: &[Compiled]) -> (String, usize) {
    match kind {
        Kind::Word if is_word_part(name) => replace_units(xml, re!(W_P), re!(W_T), true, rules),
        Kind::Slides if is_slide_part(name) => replace_units(xml, re!(A_P), re!(A_T), false, rules),
        Kind::Excel if name == "xl/sharedStrings.xml" => replace_units(xml, re!(X_SI), re!(X_T), true, rules),
        Kind::Excel if name.starts_with("xl/worksheets/") && name.ends_with(".xml") => {
            replace_units(xml, re!(X_IS), re!(X_T), true, rules)
        }
        Kind::Odf if name == "content.xml" || name == "styles.xml" => {
            let (x, n1) = odf_units(xml, re!(O_P), rules);
            let (x, n2) = odf_units(&x, re!(O_H), rules);
            (x, n1 + n2)
        }
        _ => (xml.to_string(), 0),
    }
}

fn process_zip_doc(kind: Kind, path: &Path, meta: &fs::Metadata, rules: &[Compiled], o: &Request) -> Res<Outcome> {
    let mut ar = zip::ZipArchive::new(fs::File::open(path)?)?;
    // (name, data, is_dir, compression) — order and compression are preserved (matters for ODF: mimetype)
    let mut items: Vec<(String, Vec<u8>, bool, CompressionMethod)> = Vec::new();
    let mut total = 0usize;
    for i in 0..ar.len() {
        let mut f = ar.by_index(i)?;
        let name = f.name().to_string();
        let is_dir = f.is_dir();
        let method = f.compression();
        let mut buf = Vec::new();
        if !is_dir {
            f.read_to_end(&mut buf)?;
        }
        if !is_dir {
            if let Ok(xml) = std::str::from_utf8(&buf) {
                let (new, n) = process_part(kind, &name, xml, rules);
                if n > 0 {
                    total += n;
                    buf = new.into_bytes();
                }
            }
        }
        items.push((name, buf, is_dir, method));
    }
    drop(ar);
    if total == 0 {
        return Ok(Outcome::Unchanged);
    }
    if !o.dry {
        let tmp = with_suffix(path, ".tmp~");
        {
            let mut zw = zip::ZipWriter::new(fs::File::create(&tmp)?);
            for (name, data, is_dir, method) in &items {
                let opts = SimpleFileOptions::default().compression_method(*method);
                if *is_dir {
                    zw.add_directory(name.as_str(), opts)?;
                } else {
                    zw.start_file(name.as_str(), opts)?;
                    zw.write_all(data)?;
                }
            }
            zw.finish()?;
        }
        finish_write(path, &tmp, meta, o.backup)?;
    }
    Ok(Outcome::Changed(total))
}

// ───────────── PDF (experimental, see README) ─────────────

#[cfg(feature = "pdf")]
mod pdf_impl {
    use super::*;
    use lopdf::{content::Content, Document, Object};

    fn latin1(b: &[u8]) -> String {
        b.iter().map(|&c| c as char).collect()
    }

    fn from_latin1(s: &str) -> Option<Vec<u8>> {
        let mut v = Vec::with_capacity(s.len());
        for ch in s.chars() {
            let c = ch as u32;
            if c > 255 {
                return None;
            }
            v.push(c as u8);
        }
        Some(v)
    }

    /// Edits the strings of one Tj/TJ operator. Strings with NUL bytes (2-byte
    /// Identity-H fonts) are left alone: their encoding is unknown.
    pub(super) fn edit_strings(strs: Vec<&mut Vec<u8>>, rules: &[Compiled]) -> usize {
        if strs.is_empty() || strs.iter().any(|s| s.contains(&0)) {
            return 0;
        }
        let mut runs: Vec<String> = strs.iter().map(|s| latin1(s.as_slice())).collect();
        let total = apply_to_runs(&mut runs, rules);
        if total == 0 {
            return 0;
        }
        let mut bytes = Vec::with_capacity(runs.len());
        for r in &runs {
            match from_latin1(r) {
                Some(b) => bytes.push(b),
                None => return 0,
            }
        }
        for (s, b) in strs.into_iter().zip(bytes) {
            *s = b;
        }
        total
    }

    pub fn process(path: &Path, meta: &fs::Metadata, rules: &[Compiled], o: &Request) -> Res<Outcome> {
        let mut doc = Document::load(path)?;
        if doc.is_encrypted() {
            return Ok(Outcome::Skipped("skip.encrypted".into()));
        }
        let mut total = 0usize;
        let pages: Vec<_> = doc.get_pages().into_values().collect();
        for page_id in pages {
            let data = match doc.get_page_content(page_id) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let mut content = match Content::decode(&data) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let mut page_n = 0usize;
            for op in content.operations.iter_mut() {
                match op.operator.as_str() {
                    "Tj" | "'" | "\"" => {
                        if let Some(Object::String(v, _)) =
                            op.operands.iter_mut().rev().find(|x| matches!(**x, Object::String(..)))
                        {
                            page_n += edit_strings(vec![v], rules);
                        }
                    }
                    "TJ" => {
                        if let Some(Object::Array(arr)) = op.operands.first_mut() {
                            let strs: Vec<&mut Vec<u8>> = arr
                                .iter_mut()
                                .filter_map(|x| if let Object::String(v, _) = x { Some(v) } else { None })
                                .collect();
                            page_n += edit_strings(strs, rules);
                        }
                    }
                    _ => {}
                }
            }
            if page_n > 0 {
                let bytes = content.encode()?;
                doc.change_page_content(page_id, bytes)?;
                total += page_n;
            }
        }
        if total == 0 {
            return Ok(Outcome::Unchanged);
        }
        if !o.dry {
            doc.compress();
            let tmp = with_suffix(path, ".tmp~");
            doc.save(&tmp)?;
            finish_write(path, &tmp, meta, o.backup)?;
        }
        Ok(Outcome::Changed(total))
    }
}

#[cfg(feature = "pdf")]
fn process_pdf(path: &Path, meta: &fs::Metadata, rules: &[Compiled], o: &Request) -> Res<Outcome> {
    pdf_impl::process(path, meta, rules, o)
}

#[cfg(not(feature = "pdf"))]
fn process_pdf(_p: &Path, _m: &fs::Metadata, _r: &[Compiled], _o: &Request) -> Res<Outcome> {
    Ok(Outcome::Skipped("skip.no_pdf".into()))
}

// ───────────── folder walk ─────────────

fn glob(p: &[char], t: &[char]) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn parse_masks(mask: &str) -> Vec<Vec<char>> {
    let masks: Vec<Vec<char>> = mask
        .split(|c: char| c == ';' || c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase().chars().collect())
        .collect();
    if masks.is_empty() {
        vec![vec!['*']]
    } else {
        masks
    }
}

fn process_file(path: &Path, rules: &[Compiled], o: &Request) -> Result<Outcome, String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if o.max_size_mb > 0 && meta.len() > o.max_size_mb.saturating_mul(1024 * 1024) {
        return Ok(Outcome::Skipped(format!("skip.too_big\u{1f}{}", o.max_size_mb)));
    }
    let r = if let Some(kind) = doc_kind(path) {
        process_zip_doc(kind, path, &meta, rules, o)
    } else if is_pdf(path) {
        process_pdf(path, &meta, rules, o)
    } else {
        process_text(path, &meta, rules, o)
    };
    r.map_err(|e| e.to_string())
}

pub fn run<F>(o: &Request, emit: F) -> Result<Stats, String>
where
    F: Fn(FileEvent) + Sync,
{
    let rules = compile(&o.rules, o)?;
    let root = Path::new(&o.folder);
    if !root.is_dir() {
        return Err("err.folder_not_found".into());
    }
    let masks = parse_masks(&o.mask);
    let depth = if o.recursive { usize::MAX } else { 1 };

    let files: Vec<PathBuf> = WalkDir::new(root)
        .max_depth(depth)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| !(e.file_type().is_dir() && e.file_name() == ".git"))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            let name: Vec<char> = e.file_name().to_string_lossy().to_lowercase().chars().collect();
            masks.iter().any(|m| glob(m, &name))
        })
        .map(|e| e.into_path())
        .collect();

    let changed = AtomicUsize::new(0);
    let replaced = AtomicUsize::new(0);
    let skipped = AtomicUsize::new(0);
    let errors = AtomicUsize::new(0);

    files.par_iter().for_each(|p| {
        let path = p.to_string_lossy().to_string();
        match process_file(p, &rules, o) {
            Ok(Outcome::Unchanged) => {}
            Ok(Outcome::Changed(n)) => {
                changed.fetch_add(1, Ordering::Relaxed);
                replaced.fetch_add(n, Ordering::Relaxed);
                emit(FileEvent { path, status: "ok".into(), count: n, info: String::new() });
            }
            Ok(Outcome::Skipped(why)) => {
                skipped.fetch_add(1, Ordering::Relaxed);
                emit(FileEvent { path, status: "skipped".into(), count: 0, info: why });
            }
            Err(e) => {
                errors.fetch_add(1, Ordering::Relaxed);
                emit(FileEvent { path, status: "error".into(), count: 0, info: e });
            }
        }
    });

    Ok(Stats {
        files: files.len(),
        changed: changed.load(Ordering::Relaxed),
        replaced: replaced.load(Ordering::Relaxed),
        skipped: skipped.load(Ordering::Relaxed),
        errors: errors.load(Ordering::Relaxed),
    })
}

// ───────────── rules file ─────────────

fn unesc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.peek() {
                Some('n') => { out.push('\n'); it.next(); }
                Some('t') => { out.push('\t'); it.next(); }
                Some('\\') => { out.push('\\'); it.next(); }
                _ => out.push(c),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// One rule per line: `find<TAB>replace`; `\n`, `\t` allowed; `#` starts a comment.
pub fn load_rules(path: &str, regex: bool) -> Result<Vec<Rule>, String> {
    let data = fs::read(path).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&data);
    let text = text.trim_start_matches('\u{feff}');
    let mut rules = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let (f, r) = line.split_once('\t').unwrap_or((line, ""));
        let (f, r) = if regex { (f.to_string(), r.to_string()) } else { (unesc(f), unesc(r)) };
        rules.push(Rule { find: f, repl: r });
    }
    if rules.is_empty() {
        return Err("err.no_rules_in_file".into());
    }
    Ok(rules)
}

// ───────────── tests ─────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> Request {
        Request { case_sensitive: true, ..Default::default() }
    }
    fn rule(f: &str, r: &str) -> Vec<Rule> {
        vec![Rule { find: f.into(), repl: r.into() }]
    }

    #[test]
    fn plain_keeps_edge_spaces() {
        let c = compile(&rule("мой текст ", "X"), &req()).unwrap();
        let (s, n) = apply_rules("мой текст. и мой текст .", &c, "\n");
        assert_eq!((s.as_str(), n), ("мой текст. и X.", 1));
    }

    #[test]
    fn ignore_ws_matches_any_whitespace() {
        let o = Request { ignore_ws: true, ..req() };
        let c = compile(&rule("a   b", "Z"), &o).unwrap();
        assert_eq!(apply_rules("a\n  b", &c, "\n"), ("Z".to_string(), 1));
    }

    #[test]
    fn newline_matches_crlf_and_replacement_follows_file_style() {
        let c = compile(&rule("x\ny", "1\n2"), &req()).unwrap();
        assert_eq!(apply_rules("x\r\ny", &c, "\r\n"), ("1\r\n2".to_string(), 1));
    }

    #[test]
    fn regex_groups() {
        let o = Request { regex: true, ..req() };
        let c = compile(&rule(r"(\w+)@old", "$1@new"), &o).unwrap();
        assert_eq!(apply_rules("bob@old", &c, "\n"), ("bob@new".to_string(), 1));
    }

    #[test]
    fn dollar_is_literal_in_plain_mode() {
        let c = compile(&rule("a", "$1"), &req()).unwrap();
        assert_eq!(apply_rules("a", &c, "\n").0, "$1");
    }

    #[test]
    fn runs_replacement_across_runs() {
        let c = compile(&rule("lo wor", "X"), &req()).unwrap();
        let runs = vec!["Hel".to_string(), "lo wor".to_string(), "ld".to_string()];
        let (out, n) = replace_in_runs(&runs, &c[0]);
        assert_eq!(n, 1);
        assert_eq!(out.concat(), "HelXld");
        let runs = vec!["Hello w".to_string(), "orld".to_string()];
        let (out, _) = replace_in_runs(&runs, &c[0]);
        assert_eq!(out, vec!["HelX".to_string(), "ld".to_string()]);
    }

    #[test]
    fn docx_part_entities_and_split_runs() {
        let xml = "<w:p><w:r><w:t>Привет </w:t></w:r><w:r><w:t>мир</w:t></w:r><w:r><w:t> &amp; ok</w:t></w:r></w:p>";
        let c = compile(&rule("Привет мир", "Пока"), &req()).unwrap();
        let (out, n) = process_part(Kind::Word, "word/document.xml", xml, &c);
        assert_eq!(n, 1);
        assert!(out.contains("Пока"));
        assert!(out.contains("&amp; ok"));
    }

    #[test]
    fn self_closing_paragraph_does_not_merge_neighbours() {
        let xml = r#"<w:p w:rsidR="1"/><w:p><w:r><w:t>aXb</w:t></w:r></w:p>"#;
        let c = compile(&rule("a", "1"), &req()).unwrap();
        let (out, n) = process_part(Kind::Word, "word/document.xml", xml, &c);
        assert_eq!(n, 1);
        assert!(out.starts_with(r#"<w:p w:rsidR="1"/>"#));
    }

    #[test]
    fn odf_text_split_by_span() {
        let xml = "<text:p>Hello <text:span>wor</text:span>ld</text:p>";
        let c = compile(&rule("lo wor", "X"), &req()).unwrap();
        let (out, n) = process_part(Kind::Odf, "content.xml", xml, &c);
        assert_eq!(n, 1);
        assert_eq!(out, "<text:p>HelX<text:span></text:span>ld</text:p>");
    }

    #[test]
    fn xlsx_shared_strings_rich_text() {
        let xml = "<sst><si><t>Foo</t></si><si><r><t>Ba</t></r><r><t>r</t></r></si></sst>";
        let c = compile(&rule("Bar", "Q"), &req()).unwrap();
        let (out, n) = process_part(Kind::Excel, "xl/sharedStrings.xml", xml, &c);
        assert_eq!(n, 1);
        assert!(out.contains(">Q</t>") && out.contains("Foo"));
    }

    #[test]
    fn pptx_slide_runs() {
        let xml = "<a:p><a:r><a:t>Hel</a:t></a:r><a:r><a:t>lo</a:t></a:r></a:p>";
        let c = compile(&rule("Hello", "Hi"), &req()).unwrap();
        let (out, n) = process_part(Kind::Slides, "ppt/slides/slide1.xml", xml, &c);
        assert_eq!(n, 1);
        assert!(out.contains("<a:t>Hi</a:t>") && !out.contains("xml:space"));
    }

    #[cfg(feature = "pdf")]
    #[test]
    fn pdf_strings_across_tj_pieces() {
        let c = compile(&rule("Hello", "Hi"), &req()).unwrap();
        let mut a = b"Hel".to_vec();
        let mut b = b"lo".to_vec();
        assert_eq!(pdf_impl::edit_strings(vec![&mut a, &mut b], &c), 1);
        assert_eq!(a, b"Hi".to_vec());
        assert!(b.is_empty());
        let mut z = vec![0u8, 72, 0, 105]; // Identity-H — must stay untouched
        assert_eq!(pdf_impl::edit_strings(vec![&mut z], &c), 0);
    }

    #[test]
    fn unescape_entities() {
        assert_eq!(xml_unescape("a &amp; b &#1055; &#x41; &foo"), "a & b П A &foo");
    }

    #[test]
    fn masks() {
        let m = parse_masks("*.HTML; *.txt");
        let t = |s: &str| {
            let v: Vec<char> = s.to_lowercase().chars().collect();
            m.iter().any(|p| glob(p, &v))
        };
        assert!(t("Index.html") && t("a.TXT") && !t("a.rs"));
    }

    #[test]
    fn cp1251_roundtrip() {
        let bytes = [0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2]; // "Привет"
        let (s, enc) = decode(&bytes);
        assert_eq!(s, "Привет");
        assert_eq!(encode(&s, &enc).unwrap(), bytes);
    }
}
