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
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;
use crate::flow::{replace as flow_replace, Rule as FlowRule, Fmt, plan_match};
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
#[serde(rename_all = "camelCase", default)]
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
    /// the pattern may match a paragraph break (or a tab), so the flow is needed
    spans: bool,
    /// the pattern mentions a tab: tabs at the start/end of a paragraph must be visible to it
    edge_blanks: bool,
}

impl Compiled {
    fn expand(&self, caps: &Captures, nl: &str) -> String {
        if self.is_regex {
            let mut s = String::new();
            caps.expand(&self.repl, &mut s);
            if s.contains('\n') {
                s.replace("\r\n", "\n").replace('\n', nl)
            } else {
                s
            }
        } else {
            self.repl.replace('\n', nl)
        }
    }
}

/// Horizontal blanks: space, tab, no-break and other Unicode spaces. Documents keep such
/// invisible characters at the end of a line (a line ending in two tabs, a trailing space);
/// they are lost when the text is copied into the "Find" field.
const BLANK_CLASS: &str = r"[ \t\x{00A0}\x{1680}\x{2000}-\x{200B}\x{202F}\x{205F}\x{3000}\x{FEFF}]";

fn is_blank(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\u{00A0}' | '\u{1680}' | '\u{2000}'..='\u{200B}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
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
            // Exact match: the blanks at the very start and end of the text are kept, but around
            // a line break any blanks are tolerated (see BLANK_CLASS), so a line that ends in
            // trailing spaces or tabs in the document still matches. \n also matches \r\n.
            let lines: Vec<&str> = r.find.split('\n').collect();
            let last = lines.len() - 1;
            let parts: Vec<String> = lines
                .iter()
                .enumerate()
                .map(|(k, l)| {
                    let mut l = l.trim_end_matches('\r');
                    if k > 0 {
                        l = l.trim_start_matches(is_blank);
                    }
                    if k < last {
                        l = l.trim_end_matches(is_blank);
                    }
                    regex::escape(l)
                })
                .collect();
            let sep = format!(r"{BLANK_CLASS}*\r?\n{BLANK_CLASS}*");
            parts.join(sep.as_str())
        };
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(!o.case_sensitive)
            .multi_line(true)
            .build()
            .map_err(|e| format!("err.regex\u{1f}{n}\u{1f}{e}"))?;
        // a find text copied together with a trailing tab (or a regex such as `\t+$`) has to see
        // the tabs that end a paragraph; without a tab in the pattern they stay untouched
        let edge_blanks = r.find.contains('\t')
            || (o.regex && (r.find.contains("\\t") || r.find.contains("\\s") || r.find.contains("\\x09")));
        out.push(Compiled {
            re,
            repl: if o.regex { r.repl.clone() } else { r.repl.replace("\r\n", "\n") },
            is_regex: o.regex,
            // a plain pattern needs the flow only when it contains a line break or a tab
            // (the per-paragraph path does not see tabs); `\s+` of the "ignore whitespace"
            // mode and any regex may cross a break as well
            spans: o.regex || o.ignore_ws || r.find.contains('\n') || r.find.contains('\t'),
            edge_blanks,
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

#[derive(Clone, Copy, PartialEq)]
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

pub(crate) fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub(crate) fn xml_unescape(s: &str) -> String {
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
    replace_in_runs_nl(runs, c, " ")
}

fn replace_in_runs_nl(runs: &[String], c: &Compiled, nl: &str) -> (Vec<String>, usize) {
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
        out[owner].push_str(&c.expand(m, nl));
        cur = w.end();
    }
    push_range(&full, &bounds, &mut out, cur, full.len());
    (out, caps.len())
}

fn apply_to_runs(runs: &mut Vec<String>, rules: &[Compiled]) -> usize {
    apply_to_runs_nl(runs, rules, " ")
}

fn apply_to_runs_nl(runs: &mut Vec<String>, rules: &[Compiled], nl: &str) -> usize {
    let mut total = 0usize;
    for c in rules {
        let (r, n) = replace_in_runs_nl(runs, c, nl);
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
    // Excel keeps its per-cell path: a match never crosses cells.
    if kind == Kind::Excel {
        if name == "xl/sharedStrings.xml" {
            return replace_units(xml, re!(X_SI), re!(X_T), true, rules);
        }
        if name.starts_with("xl/worksheets/") && name.ends_with(".xml") {
            return replace_units(xml, re!(X_IS), re!(X_T), true, rules);
        }
        return (xml.to_string(), 0);
    }
    let fmt = match kind {
        Kind::Word => Fmt::Word,
        Kind::Slides => Fmt::Slides,
        Kind::Odf => Fmt::Odf,
        Kind::Excel => unreachable!(),
    };
    let ok_name = match kind {
        Kind::Word => is_word_part(name),
        Kind::Slides => is_slide_part(name),
        Kind::Odf => name == "content.xml" || name == "styles.xml",
        Kind::Excel => false,
    };
    if !ok_name {
        return (xml.to_string(), 0);
    }
    if rules.iter().any(|c| c.spans) {
        let rules: Vec<FlowRule> = rules.iter().map(|c| FlowRule { re: &c.re, repl: &c.repl, is_regex: c.is_regex, edge_blanks: c.edge_blanks }).collect();
        flow_replace(xml, fmt, &rules)
    } else {
        match kind {
            Kind::Word => replace_units(xml, re!(W_P), re!(W_T), true, rules),
            Kind::Slides => replace_units(xml, re!(A_P), re!(A_T), false, rules),
            Kind::Odf => {
                let (x, n1) = odf_units(xml, re!(O_P), rules);
                let (x, n2) = odf_units(&x, re!(O_H), rules);
                (x, n1 + n2)
            }
            Kind::Excel => unreachable!(),
        }
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

// ───────────── legacy binary formats (.doc .xls .ppt) ─────────────

/// Old binary Office files cannot be edited safely in place, and automatic conversion
/// loses fonts and tables, so they are reported as skipped.
fn is_legacy_binary(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let e = e.to_ascii_lowercase();
            e == "doc" || e == "xls" || e == "ppt"
        })
        .unwrap_or(false)
}

// ───────────── RTF: edited in place, formatting is kept ─────────────

fn is_rtf(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("rtf"))
        .unwrap_or(false)
}

fn hexval(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn rtf_encoding(cp: i64) -> Option<&'static encoding_rs::Encoding> {
    Some(match cp {
        1250 => encoding_rs::WINDOWS_1250,
        1251 => encoding_rs::WINDOWS_1251,
        1252 => encoding_rs::WINDOWS_1252,
        1253 => encoding_rs::WINDOWS_1253,
        1254 => encoding_rs::WINDOWS_1254,
        1255 => encoding_rs::WINDOWS_1255,
        1256 => encoding_rs::WINDOWS_1256,
        1257 => encoding_rs::WINDOWS_1257,
        1258 => encoding_rs::WINDOWS_1258,
        874 => encoding_rs::WINDOWS_874,
        866 => encoding_rs::IBM866,
        10000 => encoding_rs::MACINTOSH,
        10007 => encoding_rs::X_MAC_CYRILLIC,
        20866 => encoding_rs::KOI8_R,
        21866 => encoding_rs::KOI8_U,
        _ => return None,
    })
}

/// Font charset (\fcharsetN) -> code page. None = use the document default (\ansicpgN).
fn charset_cp(cs: i64) -> Option<i64> {
    match cs {
        0 | 1 => None,
        238 => Some(1250),
        204 => Some(1251),
        161 => Some(1253),
        162 => Some(1254),
        177 => Some(1255),
        178 => Some(1256),
        186 => Some(1257),
        163 => Some(1258),
        222 => Some(874),
        _ => Some(-1), // multi-byte or symbol charsets are not supported
    }
}

/// Destinations whose content is not document text.
const RTF_SKIP: &[&str] = &[
    "fonttbl", "colortbl", "stylesheet", "info", "pict", "object", "objdata", "themedata",
    "colorschememapping", "latentstyles", "datastore", "xmlnstbl", "listtable",
    "listoverridetable", "rsidtbl", "generator", "fldinst", "revtbl", "template", "private",
    "sn", "sv", "operator",
];

/// One character of document text and the bytes it occupies in the RTF source.
#[derive(Clone)]
struct RtfAtom {
    start: usize,
    end: usize,
    ch: char,
    /// \ucN in effect (number of fallback characters after \uN)
    uc: usize,
}

#[derive(Clone)]
struct RtfGroup {
    font: Option<i64>,
    uc: usize,
    skip: bool,
}

/// Splits RTF into text atoms. Segments are separated by "barriers" (paragraph, cell, ...
/// or unsupported characters): a match never spans a barrier. Formatting control words
/// are transparent, so text split by bold/italic changes is still found.
fn rtf_segments(data: &[u8]) -> Vec<Vec<RtfAtom>> {
    fn push(segs: &mut Vec<Vec<RtfAtom>>, start: usize, end: usize, ch: char, uc: usize) {
        if let Some(last) = segs.last_mut() {
            last.push(RtfAtom { start, end, ch, uc });
        }
    }
    fn barrier(segs: &mut Vec<Vec<RtfAtom>>) {
        if segs.last().map(|s| !s.is_empty()).unwrap_or(false) {
            segs.push(Vec::new());
        }
    }

    let n = data.len();
    let mut segs: Vec<Vec<RtfAtom>> = vec![Vec::new()];
    let mut stack: Vec<RtfGroup> = Vec::new();
    let mut cur = RtfGroup { font: None, uc: 1, skip: false };
    let mut ansi_cp: i64 = 1252;
    let mut font_cs: HashMap<i64, i64> = HashMap::new();
    let mut pending_font: Option<i64> = None;
    let mut group_start = false;
    let mut i = 0usize;

    while i < n {
        let b = data[i];
        match b {
            b'{' => {
                stack.push(cur.clone());
                group_start = true;
                i += 1;
            }
            b'}' => {
                if let Some(g) = stack.pop() {
                    cur = g;
                }
                group_start = false;
                i += 1;
            }
            b'\r' | b'\n' => i += 1,
            b'\\' => {
                if i + 1 >= n {
                    break;
                }
                let c = data[i + 1];
                if c.is_ascii_alphabetic() {
                    // control word: \word[-N][ ]
                    let ws = i + 1;
                    let mut j = ws;
                    while j < n && data[j].is_ascii_alphabetic() {
                        j += 1;
                    }
                    let w = std::str::from_utf8(&data[ws..j]).unwrap_or("");
                    let mut k = j;
                    let neg = k < n && data[k] == b'-';
                    if neg {
                        k += 1;
                    }
                    let ds = k;
                    while k < n && data[k].is_ascii_digit() {
                        k += 1;
                    }
                    let param: Option<i64> = if k > ds {
                        std::str::from_utf8(&data[ds..k])
                            .ok()
                            .and_then(|t| t.parse::<i64>().ok())
                            .map(|v| if neg { -v } else { v })
                    } else {
                        k = j;
                        None
                    };
                    let mut end = k;
                    if end < n && data[end] == b' ' {
                        end += 1; // delimiter
                    }
                    let first = group_start;
                    group_start = false;
                    let mut next = end;
                    if first && RTF_SKIP.contains(&w) {
                        cur.skip = true;
                    }

                    let special: Option<char> = match w {
                        "tab" => Some('\t'),
                        "emdash" => Some('\u{2014}'),
                        "endash" => Some('\u{2013}'),
                        "bullet" => Some('\u{2022}'),
                        "lquote" => Some('\u{2018}'),
                        "rquote" => Some('\u{2019}'),
                        "ldblquote" => Some('\u{201C}'),
                        "rdblquote" => Some('\u{201D}'),
                        _ => None,
                    };
                    if let Some(ch) = special {
                        if !cur.skip {
                            push(&mut segs, i, end, ch, cur.uc);
                        }
                    } else {
                        match w {
                            "f" => {
                                if let Some(p) = param {
                                    cur.font = Some(p);
                                    pending_font = Some(p);
                                }
                            }
                            "fcharset" => {
                                if let (Some(pf), Some(p)) = (pending_font, param) {
                                    font_cs.insert(pf, p);
                                }
                            }
                            "ansicpg" => {
                                if let Some(p) = param {
                                    ansi_cp = p;
                                }
                            }
                            "uc" => {
                                if let Some(p) = param {
                                    cur.uc = p.max(0) as usize;
                                }
                            }
                            "bin" => {
                                if let Some(p) = param {
                                    if p > 0 {
                                        next = (end + p as usize).min(n);
                                    }
                                }
                            }
                            "u" => {
                                if let Some(p) = param {
                                    let code = if p < 0 { p + 65536 } else { p };
                                    let mut e = end;
                                    let mut cnt = cur.uc;
                                    while cnt > 0 && e < n {
                                        let c0 = data[e];
                                        if c0 == b'\\' && e + 1 < n && data[e + 1] == b'\'' {
                                            e = (e + 4).min(n);
                                        } else if c0 == b'\\' || c0 == b'{' || c0 == b'}' {
                                            break;
                                        } else if c0 == b'\r' || c0 == b'\n' {
                                            e += 1;
                                            continue;
                                        } else {
                                            e += 1;
                                        }
                                        cnt -= 1;
                                    }
                                    if !cur.skip {
                                        match u32::try_from(code).ok().and_then(char::from_u32) {
                                            Some(ch) => push(&mut segs, i, e, ch, cur.uc),
                                            None => barrier(&mut segs),
                                        }
                                    }
                                    next = e;
                                }
                            }
                            "par" | "line" | "lbr" => {
                                if !cur.skip {
                                    push(&mut segs, i, next, '\n', cur.uc);
                                }
                            }
                            "cell" | "row" | "nestcell" | "nestrow" | "sect" | "page" | "column" => {
                                if !cur.skip {
                                    barrier(&mut segs);
                                }
                            }
                            _ => {}
                        }
                    }
                    i = next;
                } else {
                    match c {
                        b'\\' | b'{' | b'}' => {
                            if !cur.skip {
                                push(&mut segs, i, i + 2, c as char, cur.uc);
                            }
                            group_start = false;
                            i += 2;
                        }
                        b'\'' => {
                            let hv = if i + 3 < n {
                                (hexval(data[i + 2]), hexval(data[i + 3]))
                            } else {
                                (None, None)
                            };
                            if let (Some(h), Some(l)) = hv {
                                if !cur.skip {
                                    let byte = h * 16 + l;
                                    let cp = cur
                                        .font
                                        .and_then(|f| font_cs.get(&f).copied())
                                        .and_then(charset_cp)
                                        .unwrap_or(ansi_cp);
                                    match rtf_encoding(cp) {
                                        Some(enc) => {
                                            let buf = [byte];
                                            let (txt, _) = enc.decode_without_bom_handling(&buf);
                                            let mut it = txt.chars();
                                            match (it.next(), it.next()) {
                                                (Some(ch), None) if ch != '\u{FFFD}' => {
                                                    push(&mut segs, i, i + 4, ch, cur.uc)
                                                }
                                                _ => barrier(&mut segs),
                                            }
                                        }
                                        None => barrier(&mut segs),
                                    }
                                }
                                i += 4;
                            } else {
                                i += 2;
                            }
                            group_start = false;
                        }
                        b'*' => {
                            if group_start {
                                cur.skip = true; // ignorable destination
                            }
                            i += 2;
                        }
                        b'~' => {
                            if !cur.skip {
                                push(&mut segs, i, i + 2, '\u{a0}', cur.uc);
                            }
                            group_start = false;
                            i += 2;
                        }
                            b'\r' | b'\n' => {
                                // backslash + newline is a paragraph mark
                                if !cur.skip {
                                    push(&mut segs, i, i + 2, '\n', cur.uc);
                                }
                                i += 2;
                            }
                        _ => i += 2, // \- \_ \| \: ... are transparent
                    }
                }
            }
            _ => {
                if !cur.skip {
                    if b < 0x80 {
                        push(&mut segs, i, i + 1, b as char, cur.uc);
                    } else {
                        barrier(&mut segs); // raw 8-bit byte: encoding unknown
                    }
                }
                group_start = false;
                i += 1;
            }
        }
    }
    segs
}

/// True when `out` ends with a control word that is not terminated yet (`\b`, `\fs24`):
/// text appended directly would be glued to it.
fn ends_with_open_ctrl(out: &[u8]) -> bool {
    let mut j = out.len();
    while j > 0 && out[j - 1].is_ascii_digit() {
        j -= 1;
    }
    if j > 0 && out[j - 1] == b'-' {
        j -= 1;
    }
    let letters_end = j;
    while j > 0 && out[j - 1].is_ascii_alphabetic() {
        j -= 1;
    }
    if j == letters_end || j == 0 {
        return false;
    }
    let mut k = j;
    let mut backslashes = 0;
    while k > 0 && out[k - 1] == b'\\' {
        backslashes += 1;
        k -= 1;
    }
    backslashes % 2 == 1
}

/// Writes `text` as RTF: ASCII as is, everything else as \uN with `uc` fallback characters.
fn push_rtf_text(out: &mut Vec<u8>, text: &str, uc: usize) {
    for ch in text.chars() {
        match ch {
            '\\' => out.extend_from_slice(b"\\\\"),
            '{' => out.extend_from_slice(b"\\{"),
            '}' => out.extend_from_slice(b"\\}"),
            '\n' => out.extend_from_slice(b"\\line "),
            '\r' => {}
            '\t' => out.extend_from_slice(b"\\tab "),
            c if (c as u32) >= 0x20 && (c as u32) < 0x7F => out.push(c as u8),
            c => {
                let mut buf = [0u16; 2];
                for &u in c.encode_utf16(&mut buf).iter() {
                    let v = u as i32;
                    let v = if v > 32767 { v - 65536 } else { v };
                    out.extend_from_slice(format!("\\u{}", v).as_bytes());
                    if uc == 0 {
                        out.push(b' ');
                    } else {
                        for _ in 0..uc {
                            out.push(b'?');
                        }
                    }
                }
            }
        }
    }
}

fn rtf_replace(data: &[u8], rules: &[Compiled]) -> (Vec<u8>, usize) {
    let mut cur = data.to_vec();
    let mut total = 0usize;
    for c in rules {
        let segs = rtf_segments(&cur);
        let mut edits: Vec<(usize, usize, Vec<u8>)> = Vec::new();
        let mut n = 0usize;
        for seg in &segs {
            if seg.is_empty() {
                continue;
            }
            let (e, cnt) = rtf_segment_edits(&cur, seg, c);
            edits.extend(e);
            n += cnt;
        }
        if n == 0 {
            continue;
        }
        edits.sort_by_key(|e| (e.0, e.1));
        let mut out = Vec::with_capacity(cur.len());
        let mut pos = 0usize;
        for (s, e, v) in &edits {
            if *s < pos {
                continue;
            }
            out.extend_from_slice(&cur[pos..*s]);
            if !v.is_empty() && ends_with_open_ctrl(&out) {
                out.push(b' ');
            }
            out.extend_from_slice(v);
            pos = *e;
        }
        out.extend_from_slice(&cur[pos..]);
        cur = out;
        total += n;
    }
    (cur, total)
}

/// Write-back of one RTF segment for one rule. A `\par`/`\line` is now a `\n` atom, so a
/// match may span paragraphs; edits are planned line by line like the OOXML flow.
fn rtf_segment_edits(data: &[u8], seg: &[RtfAtom], r: &Compiled) -> (Vec<(usize, usize, Vec<u8>)>, usize) {
    let mut text = String::with_capacity(seg.len());
    let mut starts: Vec<usize> = Vec::with_capacity(seg.len());
    for a in seg {
        starts.push(text.len());
        text.push(a.ch);
    }
    // lines: (first atom, atom past last text char, nl atom index, byte start in text, byte length)
    let mut lines: Vec<(usize, usize, Option<usize>, usize, usize)> = Vec::new();
    let mut a = 0usize;
    let mut gs = 0usize;
    for i in 0..seg.len() {
        if seg[i].ch != '\n' {
            continue;
        }
        lines.push((a, i, Some(i), gs, starts[i] - gs));
        a = i + 1;
        gs = if i + 1 < seg.len() { starts[i + 1] } else { text.len() };
    }
    if a < seg.len() {
        lines.push((a, seg.len(), None, gs, text.len() - gs));
    }
    let caps: Vec<Captures> = r.re.captures_iter(&text).filter(|m| !m[0].is_empty()).collect();
    if caps.is_empty() {
        return (Vec::new(), 0);
    }
    let mut ops: BTreeMap<usize, Vec<(usize, usize, String)>> = BTreeMap::new();
    let mut dels: Vec<usize> = Vec::new();
    let mut ins: Vec<(usize, Vec<String>)> = Vec::new();
    for m in &caps {
        let w = m.get(0).unwrap();
        let (la, oa) = rtf_locate(&lines, w.start());
        let (lb, ob) = rtf_locate(&lines, w.end());
        let lens: Vec<usize> = (la..=lb).map(|i| lines[i].4).collect();
        let l = lines[lb];
        let suffix = text[l.3 + ob..l.3 + l.4].to_string();
        let mut p = plan_match(la, oa, lb, ob, &lens, &r.expand(m, "\n"), &suffix);
        for (k, v) in p.ops {
            ops.entry(k).or_default().extend(v);
        }
        dels.append(&mut p.dels);
        ins.append(&mut p.ins);
    }
    // a line that another match also writes into is emptied instead of being deleted
    let dl: HashSet<usize> = dels.into_iter().collect();
    let mut removed: Vec<usize> = Vec::new();
    for d in &dl {
        if let Some(list) = ops.get_mut(d) {
            let joined: String = list.iter().map(|(_, _, s)| s.as_str()).collect();
            *list = vec![(0, lines[*d].4, joined)];
        } else {
            removed.push(*d);
        }
    }
    let mut edits: Vec<(usize, usize, Vec<u8>)> = Vec::new();
    for d in removed {
        let l = lines[d];
        let nlk = match d.checked_sub(1).and_then(|p| lines[p].2) {
            Some(k) => k,
            None => continue,
        };
        let e = if l.1 > l.0 { seg[l.1 - 1].end } else { seg[nlk].end };
        let s = seg[nlk].start;
        if braces_balanced(&data[s..e]) {
            edits.push((s, e, Vec::new()));
        } else if l.1 > l.0 {
            edits.push((seg[l.0].start, seg[l.1 - 1].end, Vec::new()));
        }
    }
    for (li, list) in &ops {
        let l = lines[*li];
        let uc = seg.get(l.0).map(|a| a.uc).unwrap_or(1);
        for (ra, rb, txt) in list {
            if txt.is_empty() && ra == rb {
                continue;
            }
            let ka = l.0 + text[l.3..l.3 + ra].chars().count();
            let kb = l.0 + text[l.3..l.3 + rb].chars().count();
            if ka >= seg.len() {
                let p = seg[seg.len() - 1].end;
                let mut v = Vec::new();
                push_rtf_text(&mut v, txt, uc);
                edits.push((p, p, v));
                continue;
            }
            let mut v = Vec::new();
            push_rtf_text(&mut v, txt, uc);
            edits.push((seg[ka].start, seg[ka].end, v));
            for k in ka + 1..kb.min(seg.len()) {
                edits.push((seg[k].start, seg[k].end, Vec::new()));
            }
        }
    }
    for (anchor, extras) in ins {
        let l = lines[anchor];
        let uc = seg.get(l.0).map(|a| a.uc).unwrap_or(1);
        let mut v = Vec::new();
        let pos = match l.2 {
            Some(k) => {
                for t in extras {
                    push_rtf_text(&mut v, &t, uc);
                    v.extend_from_slice(b"\\par ");
                }
                seg[k].end
            }
            None => {
                for t in extras {
                    v.extend_from_slice(b"\\par ");
                    push_rtf_text(&mut v, &t, uc);
                }
                if l.1 > l.0 { seg[l.1 - 1].end } else { seg[l.0].start }
            }
        };
        edits.push((pos, pos, v));
    }
    (edits, caps.len())
}

fn rtf_locate(lines: &[(usize, usize, Option<usize>, usize, usize)], p: usize) -> (usize, usize) {
    let i = match lines.binary_search_by_key(&p, |l| l.3) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    (i, (p - lines[i].3).min(lines[i].4))
}

/// Deleted bytes must keep braces balanced, otherwise the RTF group structure breaks.
fn braces_balanced(b: &[u8]) -> bool {
    let mut open = 0i32;
    let mut close = 0i32;
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'{' => {
                open += 1;
                i += 1;
            }
            b'}' => {
                close += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    open == close
}

fn process_rtf(path: &Path, meta: &fs::Metadata, rules: &[Compiled], o: &Request) -> Res<Outcome> {
    let data = fs::read(path)?;
    if !data.starts_with(b"{\\rtf") {
        return Ok(Outcome::Skipped("skip.not_rtf".into()));
    }
    let (out, n) = rtf_replace(&data, rules);
    if n == 0 {
        return Ok(Outcome::Unchanged);
    }
    if !o.dry && out != data {
        write_atomic(path, &out, meta, o.backup)?;
    }
    Ok(Outcome::Changed(n))
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
    if is_legacy_binary(path) {
        return Ok(Outcome::Skipped("skip.legacy".into()));
    }
    let r = if let Some(kind) = doc_kind(path) {
        process_zip_doc(kind, path, &meta, rules, o)
    } else if is_pdf(path) {
        process_pdf(path, &meta, rules, o)
    } else if is_rtf(path) {
        process_rtf(path, &meta, rules, o)
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
            // skip Office/LibreOffice lock files
            let n = e.file_name().to_string_lossy();
            !(n.starts_with("~$") || n.starts_with(".~lock."))
        })
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

    // big stacks: the legacy-format parsers recurse deeply on some files
    let pool = rayon::ThreadPoolBuilder::new()
        .stack_size(16 * 1024 * 1024)
        .build()
        .map_err(|e| e.to_string())?;

    pool.install(|| {
        files.par_iter().for_each(|p| {
            let path = p.to_string_lossy().to_string();
            // a parser bug must not take the whole application down
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| process_file(p, &rules, o)))
                .unwrap_or_else(|_| Err("err.internal".to_string()));
            match res {
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
    fn line_break_ignores_blanks_around_it() {
        let c = compile(&rule("a\nb", "X"), &req()).unwrap();
        // spaces, tabs and no-break spaces before the break, blanks after it
        assert_eq!(apply_rules("a \t\u{a0}\n \tb", &c, "\n"), ("X".to_string(), 1));
        assert_eq!(apply_rules("a\r\n b", &c, "\r\n"), ("X".to_string(), 1));
        // blanks typed around the break in the find text do not matter either
        let c = compile(&rule("a \t\n b", "X"), &req()).unwrap();
        assert_eq!(apply_rules("a\nb", &c, "\n"), ("X".to_string(), 1));
    }

    #[test]
    fn blanks_at_the_edges_of_the_whole_text_stay_exact() {
        let c = compile(&rule("a\nb ", "X"), &req()).unwrap();
        assert_eq!(apply_rules("a\nb", &c, "\n").1, 0); // the trailing blank belongs to the text
        assert_eq!(apply_rules("a\nb ", &c, "\n"), ("X".to_string(), 1));
        let c = compile(&rule(" a\nb", "X"), &req()).unwrap();
        assert_eq!(apply_rules("a\nb", &c, "\n").1, 0);
    }

    #[test]
    fn empty_line_inside_the_find_text() {
        let c = compile(&rule("a\n\nb", "X"), &req()).unwrap();
        assert_eq!(apply_rules("a \n \nb", &c, "\n"), ("X".to_string(), 1));
        assert_eq!(apply_rules("a\nb", &c, "\n").1, 0);
    }

    #[test]
    fn word_block_with_trailing_tabs_and_spaces_is_found() {
        // the real layout: the first line ends with two tabs, the second with a space
        let xml = concat!(
            "<w:p><w:r><w:t>Contractor: A.</w:t></w:r><w:r><w:tab/></w:r><w:r><w:tab/></w:r></w:p>",
            "<w:p><w:r><w:t>Customer: B. </w:t></w:r></w:p>",
            "<w:p><w:r><w:t>Date: C</w:t></w:r></w:p>"
        );
        let c = compile(&rule("Contractor: A.\t\nCustomer: B.\nDate: C", "1\n2\n3"), &req()).unwrap();
        let (out, n) = process_part(Kind::Word, "word/document.xml", xml, &c);
        assert_eq!(n, 1);
        for s in [">1<", ">2<", ">3<"] {
            assert!(out.contains(s), "{s} missing in {out}");
        }
    }

    fn tab_count(s: &str) -> usize {
        s.matches("<w:tab/>").count()
    }

    const TWO_TABS: &str = concat!(
        "<w:p><w:r><w:t>Object: A.</w:t></w:r><w:r><w:tab/></w:r><w:r><w:tab/></w:r></w:p>",
        "<w:p><w:r><w:t>Customer: B.</w:t></w:r></w:p>"
    );

    #[test]
    fn word_trailing_tabs_copied_with_the_text_can_be_removed() {
        // "Object: A." + two tabs is copied from the document and replaced without them
        let c = compile(&rule("Object: A.\t\t", "Object: A."), &req()).unwrap();
        let (out, n) = process_part(Kind::Word, "word/document.xml", TWO_TABS, &c);
        assert_eq!(n, 1);
        assert_eq!(tab_count(&out), 0, "{out}");
        assert!(out.contains("Object: A.</w:t>"), "{out}");
        assert!(out.contains("Customer: B."), "{out}");
    }

    #[test]
    fn word_one_tab_in_the_find_removes_exactly_one_tab() {
        let c = compile(&rule("A.\t", "A."), &req()).unwrap();
        let (out, n) = process_part(Kind::Word, "word/document.xml", TWO_TABS, &c);
        assert_eq!(n, 1);
        assert_eq!(tab_count(&out), 1, "{out}");
    }

    #[test]
    fn word_trailing_tabs_stay_when_the_find_has_none() {
        // single line, and a line break between two paragraphs: the tabs are not touched
        let c = compile(&rule("Object: A.", "X"), &req()).unwrap();
        let (out, n) = process_part(Kind::Word, "word/document.xml", TWO_TABS, &c);
        assert_eq!((n, tab_count(&out)), (1, 2));
        let c = compile(&rule("Object: A.\nCustomer: B.", "X\nY"), &req()).unwrap();
        let (out, n) = process_part(Kind::Word, "word/document.xml", TWO_TABS, &c);
        assert_eq!((n, tab_count(&out)), (1, 2), "{out}");
    }

    #[test]
    fn word_regex_can_strip_trailing_tabs() {
        let o = Request { regex: true, ..req() };
        let c = compile(&rule(r"\t+$", ""), &o).unwrap();
        let (out, n) = process_part(Kind::Word, "word/document.xml", TWO_TABS, &c);
        assert_eq!(n, 1);
        assert_eq!(tab_count(&out), 0, "{out}");
    }

    #[test]
    fn word_trailing_spaces_are_matched_and_removed() {
        let xml = "<w:p><w:r><w:t xml:space=\"preserve\">Text.   </w:t></w:r></w:p>";
        let c = compile(&rule("Text.   ", "Text."), &req()).unwrap();
        let (out, n) = process_part(Kind::Word, "word/document.xml", xml, &c);
        assert_eq!(n, 1);
        assert!(out.contains(">Text.<"), "{out}");
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
    fn legacy_binary_formats_detected() {
        assert!(is_legacy_binary(Path::new("dir/Report.DOC")));
        assert!(is_legacy_binary(Path::new("a.xls")));
        assert!(is_legacy_binary(Path::new("a.ppt")));
        assert!(!is_legacy_binary(Path::new("a.docx")));
        assert!(!is_legacy_binary(Path::new("a.rtf")));
        assert!(!is_legacy_binary(Path::new("noext")));
    }

    #[test]
    fn regex_replacement_newlines_follow_file_style() {
        let o = Request { regex: true, ..req() };
        let c = compile(&rule("a", "x\ny"), &o).unwrap();
        assert_eq!(apply_rules("a", &c, "\r\n").0, "x\r\ny");
        assert_eq!(apply_rules("a", &c, "\n").0, "x\ny");
    }

    #[derive(Deserialize)]
    struct Recipe {
        id: String,
        find: String,
        repl: String,
        sample: String,
        result: String,
    }

    /// Every recipe shown in the help window must really work with this engine.
    #[test]
    fn regex_recipes_work() {
        let list: Vec<Recipe> = serde_json::from_str(include_str!("../../src/recipes.json")).unwrap();
        assert!(list.len() >= 10);
        for r in list {
            let o = Request { regex: true, case_sensitive: true, ..Default::default() };
            let c = compile(&rule(&r.find, &r.repl), &o).unwrap_or_else(|e| panic!("{}: {}", r.id, e));
            let (out, n) = apply_rules(&r.sample, &c, "\n");
            assert!(n >= 1, "recipe {} did not match its sample", r.id);
            assert_eq!(out, r.result, "recipe {}", r.id);
        }
    }

    fn rtf_text(data: &[u8]) -> String {
        rtf_segments(data).iter().flat_map(|s| s.iter().map(|a| a.ch)).collect()
    }

    fn run_rtf(src: &[u8], find: &str, repl: &str) -> (String, usize) {
        let c = compile(&rule(find, repl), &req()).unwrap();
        let (out, n) = rtf_replace(src, &c);
        (String::from_utf8_lossy(&out).to_string(), n)
    }

    #[test]
    fn rtf_plain_replacement() {
        let (out, n) = run_rtf(br"{\rtf1\ansi Hello World\par}", "World", "There");
        assert_eq!(n, 1);
        assert_eq!(out, r"{\rtf1\ansi Hello There\par}");
    }

    #[test]
    fn rtf_match_across_formatting() {
        let (out, n) = run_rtf(br"{\rtf1\ansi Hello {\b Wor}ld\par}", "World", "X");
        assert_eq!(n, 1);
        assert_eq!(out, r"{\rtf1\ansi Hello {\b X}\par}");
    }

    #[test]
    fn rtf_cyrillic_cp1251() {
        let src = br"{\rtf1\ansi\ansicpg1251\deff0{\fonttbl{\f0\fcharset204 Arial;}}\f0 \'cf\'f0\'e8\'e2\'e5\'f2 \'ec\'e8\'f0\par}";
        assert_eq!(rtf_text(src), "Привет мир\n");
        let (out, n) = run_rtf(src, "Привет", "Здравствуй");
        assert_eq!(n, 1);
        assert_eq!(rtf_text(out.as_bytes()), "Здравствуй мир\n");
        assert!(out.contains(r"\u1047?"));
    }

    #[test]
    fn rtf_control_word_gets_delimiter_before_inserted_text() {
        let (out, n) = run_rtf(br"{\rtf1\ansi \b\'41\b0 rest}", "A", "Zed");
        assert_eq!(n, 1);
        assert_eq!(out, r"{\rtf1\ansi \b Zed\b0 rest}");
    }

    #[test]
    fn rtf_unicode_escape_and_fallback() {
        let (out, n) = run_rtf(br"{\rtf1\ansi\uc1 caf\u233? x}", "caf\u{e9}", "tea");
        assert_eq!(n, 1);
        assert_eq!(out, r"{\rtf1\ansi\uc1 tea x}");
    }

    #[test]
    fn rtf_font_table_is_not_touched() {
        let (out, n) = run_rtf(br"{\rtf1{\fonttbl{\f0 Arial;}}\f0 Arial text}", "Arial", "X");
        assert_eq!(n, 1);
        assert_eq!(out, r"{\rtf1{\fonttbl{\f0 Arial;}}\f0 X text}");
    }

    #[test]
    fn rtf_match_does_not_span_paragraphs() {
        let (_, n) = run_rtf(br"{\rtf1\ansi one\par two}", "onetwo", "X");
        assert_eq!(n, 0);
    }

    #[test]
    fn rtf_special_characters_are_escaped() {
        let (out, _) = run_rtf(br"{\rtf1\ansi a}", "a", r"{x}\y");
        assert_eq!(out, r"{\rtf1\ansi \{x\}\\y}");
    }

    #[test]
    fn rtf_multiline_aligned() {
        let (out, n) = run_rtf(br"{\rtf1\ansi one\par two}", "one\ntwo", "X\nY");
        assert_eq!(n, 1);
        assert_eq!(out, r"{\rtf1\ansi X\par Y}");
    }

    #[test]
    fn rtf_multiline_fewer_lines() {
        let (out, n) = run_rtf(br"{\rtf1\ansi one\par two}", "one\ntwo", "Z");
        assert_eq!(n, 1);
        assert_eq!(out, r"{\rtf1\ansi Z}");
    }

    #[test]
    fn rtf_multiline_more_lines() {
        let (out, n) = run_rtf(br"{\rtf1\ansi one\par two}", "one\ntwo", "1\n2\n3");
        assert_eq!(n, 1);
        assert_eq!(out, r"{\rtf1\ansi 1\par 2\par 3}");
    }

    #[test]
    fn rtf_multiline_does_not_cross_cells() {
        let (out, n) = run_rtf(br"{\rtf1\ansi \cell one\par two\cell \cell three\par four\cell}", "two\nthree", "X");
        assert_eq!(n, 0);
        assert_eq!(out, r"{\rtf1\ansi \cell one\par two\cell \cell three\par four\cell}");
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
