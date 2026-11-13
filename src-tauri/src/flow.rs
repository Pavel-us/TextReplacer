//! Search and replace across paragraph boundaries in zipped documents.
//!
//! A part (Word `document.xml`, PowerPoint `slide1.xml`, ODF `content.xml`) is read as a
//! **flow**: paragraphs become lines, `<w:br/>`/`<w:tab/>` become `\n`/`\t`, and the search
//! runs over the joined text. Every line of the result is written back into a paragraph of
//! its own, so paragraph properties and run formatting survive. Cells, tables, text boxes
//! and section breaks stay barriers: a match never crosses them.

use regex::{Captures, Regex};
use std::collections::{BTreeMap, HashSet};

use crate::engine::{xml_escape, xml_unescape};

/// Element names a part of each format family is made of.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Fmt {
    Word,
    Slides,
    Odf,
}

impl Fmt {
    fn para(&self) -> &'static [&'static str] {
        match self {
            Fmt::Word => &["w:p"],
            Fmt::Slides => &["a:p"],
            Fmt::Odf => &["text:p", "text:h"],
        }
    }
    /// Element that holds run text (ODF keeps its text between tags).
    fn text(&self) -> &'static str {
        match self {
            Fmt::Word => "w:t",
            Fmt::Slides => "a:t",
            Fmt::Odf => "",
        }
    }
    fn br(&self) -> &'static [&'static str] {
        match self {
            Fmt::Word => &["w:br", "w:cr"],
            Fmt::Slides => &["a:br"],
            Fmt::Odf => &["text:line-break"],
        }
    }
    fn tab(&self) -> &'static str {
        match self {
            Fmt::Word => "w:tab",
            Fmt::Slides => "a:tab",
            Fmt::Odf => "text:tab",
        }
    }
    fn ns(&self) -> &'static str {
        match self {
            Fmt::Word => "w",
            Fmt::Slides => "a",
            Fmt::Odf => "",
        }
    }
    fn ppr(&self) -> &'static str {
        match self {
            Fmt::Word => "w:pPr",
            Fmt::Slides => "a:pPr",
            Fmt::Odf => "",
        }
    }
    fn run(&self) -> &'static str {
        match self {
            Fmt::Word => "w:r",
            Fmt::Slides => "a:r",
            Fmt::Odf => "",
        }
    }
    fn rpr(&self) -> &'static str {
        match self {
            Fmt::Word => "w:rPr",
            Fmt::Slides => "a:rPr",
            Fmt::Odf => "",
        }
    }
    /// Replaced Word text keeps its spaces only with this attribute.
    fn preserve(&self) -> bool {
        matches!(self, Fmt::Word)
    }
    /// A match never crosses these: cells, tables, text boxes, sections.
    fn barrier(&self) -> &'static [&'static str] {
        match self {
            Fmt::Word => &["w:tc", "w:tbl", "w:tr", "w:txbxContent", "w:sdtContent", "w:sectPr"],
            Fmt::Slides => &[
                "a:tc",
                "a:tbl",
                "a:tr",
                "a:txBody",
                "p:sp",
                "p:pic",
                "p:graphicFrame",
                "p:grpSp",
                "p:cxnSp",
            ],
            Fmt::Odf => &[
                "text:table-cell",
                "text:table",
                "text:table-row",
                "text:list",
                "text:list-item",
                "text:section",
                "text:note-body",
                "table:table-cell",
                "table:table",
                "table:table-row",
                "draw:frame",
                "draw:text-box",
            ],
        }
    }
    /// Paragraphs holding these are emptied instead of being deleted.
    fn keep(&self) -> &'static [&'static str] {
        match self {
            Fmt::Word => &[
                "<w:drawing",
                "<w:pict",
                "<w:object",
                "<w:sectPr",
                "<w:bookmarkStart",
                "<w:bookmarkEnd",
                "<w:commentR",
                "<w:fldSimple",
                "<w:fldChar",
                "<w:instrText",
                "<w:footnoteReference",
                "<w:endnoteReference",
                "<w:ins ",
                "<w:del",
                "<w:moveFrom",
                "<w:moveTo",
                "<m:oMath",
                "<w:txbxContent",
                "<w:sym",
                "<w:noBreakHyphen",
                "<w:permStart",
                "<w:tbl",
            ],
            Fmt::Slides => &[
                "<a:blip",
                "<p:ph",
                "<a:fld",
                "<p:graphicFrame",
                "<p:pic",
                "<a:videoFile",
                "<a:audioFile",
            ],
            Fmt::Odf => &[
                "<draw:",
                "<text:note",
                "<text:bookmark",
                "<text:reference-mark",
                "<text:index-mark",
                "<office:annotation",
                "<text:user-field-get",
                "<text:user-field-decl",
                "<text:page-number",
            ],
        }
    }
}

/// What the flow needs to know about one compiled rule.
#[derive(Clone, Copy)]
pub struct Rule<'a> {
    pub re: &'a Regex,
    pub repl: &'a str,
    pub is_regex: bool,
    /// Tabs and breaks at the start/end of a paragraph are part of its text for this rule
    /// (the find text mentions a tab). Otherwise they are invisible and kept as they are.
    pub edge_blanks: bool,
}

impl Rule<'_> {
    /// Replacement text of one match; newlines stay `\n` and become breaks on write.
    fn expand(&self, caps: &Captures) -> String {
        if self.is_regex {
            let mut s = String::new();
            caps.expand(self.repl, &mut s);
            s.replace("\r\n", "\n")
        } else {
            self.repl.to_string()
        }
    }
}

struct Tag {
    span: (usize, usize),
    name: (usize, usize),
    close: bool,
    selfc: bool,
}

fn tag_name<'a>(xml: &'a str, t: &Tag) -> &'a str {
    &xml[t.name.0..t.name.1]
}

/// All start and end tags of a part, in document order.
fn scan_tags(xml: &str) -> Vec<Tag> {
    let b = xml.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &xml[i..];
        if rest.starts_with("<!--") {
            match rest.find("-->") {
                Some(e) => i += e + 3,
                None => break,
            }
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            match rest.find("]]>") {
                Some(e) => i += e + 3,
                None => break,
            }
            continue;
        }
        if rest.starts_with("<?") {
            match rest.find("?>") {
                Some(e) => i += e + 2,
                None => break,
            }
            continue;
        }
        if rest.starts_with("<!") {
            match rest.find('>') {
                Some(e) => i += e + 1,
                None => break,
            }
            continue;
        }
        let mut j = i + 1;
        let mut q: Option<u8> = None;
        loop {
            if j >= b.len() {
                break;
            }
            let c = b[j];
            if let Some(cc) = q {
                if c == cc {
                    q = None;
                }
            } else if c == b'"' || c == b'\'' {
                q = Some(c);
            } else if c == b'>' {
                break;
            }
            j += 1;
        }
        if j >= b.len() {
            break;
        }
        let inner = &xml[i + 1..j];
        let close = inner.starts_with('/');
        let selfc = inner.ends_with('/');
        let nm = inner.trim_start_matches('/').trim_end_matches('/');
        let ne = nm.find(char::is_whitespace).unwrap_or(nm.len());
        let ns = i + 1 + usize::from(close);
        out.push(Tag { span: (i, j + 1), name: (ns, ns + ne), close, selfc });
        i = j + 1;
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotKind {
    Text,
    Break,
    Tab,
}

/// One editable piece of a paragraph: run text, a line break or a tab.
struct Slot {
    span: (usize, usize),
    text: String,
    kind: SlotKind,
}

struct Line {
    /// the whole paragraph element
    span: (usize, usize),
    /// its opening tag
    open: (usize, usize),
    slots: Vec<Slot>,
    /// index range of `slots` that form the matchable text (leading/trailing
    /// whitespace-only slots, e.g. trailing `<w:tab/>`, are excluded)
    usable: (usize, usize),
    text: String,
    /// false when the paragraph holds something that must not disappear
    deletable: bool,
    close_name: String,
}

struct Flow {
    tags: Vec<Tag>,
    lines: Vec<Line>,
    /// paragraphs of one uninterrupted flow: index ranges into `lines`
    segs: Vec<(usize, usize)>,
}

/// Paragraphs of a part as editable lines. A paragraph that contains other paragraphs
/// (a text box) is skipped as a whole: its inner paragraphs are lines of their own.
fn parse_flow(xml: &str, fmt: Fmt, edge_blanks: bool) -> Flow {
    let tags = scan_tags(xml);
    let mut lines: Vec<Line> = Vec::new();
    let mut segs: Vec<(usize, usize)> = Vec::new();
    let mut seg_start = 0usize;
    let mut barrier = false;
    let mut stack: Vec<(usize, bool)> = Vec::new();
    for ti in 0..tags.len() {
        let nm = tag_name(xml, &tags[ti]);
        if fmt.barrier().contains(&nm) {
            barrier = true;
            continue;
        }
        if !fmt.para().contains(&nm) {
            continue;
        }
        if tags[ti].close {
            if let Some((oi, nested)) = stack.pop() {
                if !nested {
                    if barrier && lines.len() > seg_start {
                        segs.push((seg_start, lines.len()));
                        seg_start = lines.len();
                    }
                    barrier = false;
                    lines.push(make_line(xml, &tags, fmt, oi, ti, edge_blanks));
                }
            }
        } else if tags[ti].selfc {
            if barrier && lines.len() > seg_start {
                segs.push((seg_start, lines.len()));
                seg_start = lines.len();
            }
            barrier = false;
            lines.push(Line {
                span: tags[ti].span,
                open: tags[ti].span,
                slots: Vec::new(),
                usable: (0, 0),
                text: String::new(),
                deletable: true,
                close_name: nm.to_string(),
            });
        } else {
            if let Some(p) = stack.last_mut() {
                p.1 = true;
            }
            stack.push((ti, false));
        }
    }
    if lines.len() > seg_start {
        segs.push((seg_start, lines.len()));
    }
    Flow { tags, lines, segs }
}

/// Collects the editable slots of the paragraph between tags `oi` and `ci`.
fn make_line(xml: &str, tags: &[Tag], fmt: Fmt, oi: usize, ci: usize, edge_blanks: bool) -> Line {
    let span = (tags[oi].span.0, tags[ci].span.1);
    let mut slots: Vec<Slot> = Vec::new();
    if fmt == Fmt::Odf {
        let mut last = tags[oi].span.1;
        for tj in oi + 1..ci {
            let t = &tags[tj];
            let nm = tag_name(xml, t);
            if t.span.0 > last {
                slots.push(Slot {
                    span: (last, t.span.0),
                    text: xml_unescape(&xml[last..t.span.0]),
                    kind: SlotKind::Text,
                });
            }
            last = t.span.1;
            if fmt.br().contains(&nm) && t.selfc {
                slots.push(Slot { span: t.span, text: "\n".into(), kind: SlotKind::Break });
            } else if nm == fmt.tab() && t.selfc {
                slots.push(Slot { span: t.span, text: "\t".into(), kind: SlotKind::Tab });
            } else if nm == "text:s" && t.selfc {
                let n = attr_int(xml, t, "text:c").unwrap_or(1).clamp(1, 4096) as usize;
                slots.push(Slot { span: t.span, text: " ".repeat(n), kind: SlotKind::Text });
            }
        }
        if tags[ci].span.0 > last {
            slots.push(Slot {
                span: (last, tags[ci].span.0),
                text: xml_unescape(&xml[last..tags[ci].span.0]),
                kind: SlotKind::Text,
            });
        }
    } else {
        let mut tj = oi + 1;
        while tj < ci {
            let t = &tags[tj];
            let nm = tag_name(xml, t);
            if nm == fmt.text() && !t.close {
                if t.selfc {
                    slots.push(Slot { span: t.span, text: String::new(), kind: SlotKind::Text });
                } else {
                    let mut k = tj + 1;
                    while k < ci && !(tag_name(xml, &tags[k]) == nm && tags[k].close) {
                        k += 1;
                    }
                    if k < ci {
                        slots.push(Slot {
                            span: (t.span.0, tags[k].span.1),
                            text: xml_unescape(&xml[t.span.1..tags[k].span.0]),
                            kind: SlotKind::Text,
                        });
                        tj = k;
                    }
                }
            } else if fmt.br().contains(&nm) && t.selfc {
                slots.push(Slot { span: t.span, text: "\n".into(), kind: SlotKind::Break });
            } else if nm == fmt.tab() && t.selfc {
                slots.push(Slot { span: t.span, text: "\t".into(), kind: SlotKind::Tab });
            }
            tj += 1;
        }
    }
    let mut first = 0usize;
    let mut last = slots.len();
    if !edge_blanks {
        while first < slots.len() && is_ws_slot(&slots[first]) {
            first += 1;
        }
        while last > first && is_ws_slot(&slots[last - 1]) {
            last -= 1;
        }
    }
    let usable = (first, last);
    let text: String = slots[usable.0..usable.1].iter().map(|s| s.text.as_str()).collect();
    let body = &xml[span.0..span.1];
    let deletable = !fmt.keep().iter().any(|p| body.contains(p));
    let close_name = tag_name(xml, &tags[ci]).to_string();
    Line { span, open: tags[oi].span, slots, usable, text, deletable, close_name }
}

/// A slot that holds only tab/break characters: not matchable, but preserved on write-back.
fn is_ws_slot(s: &Slot) -> bool {
    !s.text.is_empty() && s.text.chars().all(|c| c == '\t' || c == '\n')
}

/// Integer attribute of a tag: `text:c` of `<text:s text:c="3"/>`.
fn attr_int(xml: &str, t: &Tag, name: &str) -> Option<i64> {
    let inner = &xml[t.span.0..t.span.1];
    let pat = format!("{name}=\"");
    let i = inner.find(&pat)? + pat.len();
    let j = i + inner[i..].find('"')?;
    inner[i..j].parse().ok()
}

// ───────────── write-back ─────────────

/// Where a match that covers lines `la..=lb` is written back, line by line.
pub(crate) struct Plan {
    /// line -> (from, to, inserted text) inside the text of that line
    pub(crate) ops: BTreeMap<usize, Vec<(usize, usize, String)>>,
    /// lines that disappear
    pub(crate) dels: Vec<usize>,
    /// new paragraphs to insert after a line
    pub(crate) ins: Vec<(usize, Vec<String>)>,
}

/// `lens[i]` is the length of line `la + i`; `suffix` is what is left of the last line.
pub(crate) fn plan_match(la: usize, oa: usize, lb: usize, ob: usize, lens: &[usize], repl: &str, suffix: &str) -> Plan {
    let k = lb - la + 1;
    let parts: Vec<&str> = repl.split('\n').collect();
    let m = parts.len();
    let mut p = Plan { ops: BTreeMap::new(), dels: Vec::new(), ins: Vec::new() };
    if k == 1 {
        // inside one paragraph: newlines become soft breaks, the paragraph is not split
        p.ops.insert(la, vec![(oa, ob, repl.to_string())]);
        return p;
    }
    if m <= k {
        for i in 0..m {
            let ra = if i == 0 { oa } else { 0 };
            let rb = if m == k && i + 1 == m { ob } else { lens[i] };
            let mut txt = parts[i].to_string();
            if m < k && i + 1 == m {
                txt.push_str(suffix); // the paragraph that held the tail is deleted
            }
            p.ops.entry(la + i).or_default().push((ra, rb, txt));
        }
        for i in m..k {
            p.dels.push(la + i);
        }
    } else {
        for i in 0..k {
            let ra = if i == 0 { oa } else { 0 };
            p.ops.entry(la + i).or_default().push((ra, lens[i], parts[i].to_string()));
        }
        let mut extra: Vec<String> = parts[k..].iter().map(|s| s.to_string()).collect();
        if let Some(last) = extra.last_mut() {
            last.push_str(suffix);
        }
        p.ins.push((lb, extra));
    }
    p
}

struct Edit {
    start: usize,
    end: usize,
    text: String,
}

fn apply_edits(xml: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|e| (e.start, e.end));
    let mut out = String::with_capacity(xml.len());
    let mut pos = 0usize;
    for e in edits {
        if e.start < pos {
            continue; // overlaps an edit already applied
        }
        out.push_str(&xml[pos..e.start]);
        out.push_str(&e.text);
        pos = e.end;
    }
    out.push_str(&xml[pos..]);
    out
}

/// Text of every slot of the line after replacing the given ranges.
fn splice_line(line: &Line, splices: &[(usize, usize, String)]) -> Vec<String> {
    let usable = &line.slots[line.usable.0..line.usable.1];
    if usable.is_empty() {
        return Vec::new();
    }
    let mut bounds = Vec::with_capacity(usable.len());
    let mut pos = 0usize;
    for s in usable {
        bounds.push((pos, pos + s.text.len()));
        pos += s.text.len();
    }
    let mut out = vec![String::new(); usable.len()];
    let mut cur = 0usize;
    for (a, b, ins) in splices {
        push_part(&line.text, &bounds, &mut out, cur, *a);
        let owner = bounds
            .iter()
            .position(|&(s, e)| *a >= s && *a < e)
            .unwrap_or(out.len() - 1);
        out[owner].push_str(ins);
        cur = *b;
    }
    push_part(&line.text, &bounds, &mut out, cur, line.text.len());
    out
}

/// Copies `full[a..b]` into the slots that own those characters.
fn push_part(full: &str, bounds: &[(usize, usize)], out: &mut [String], a: usize, b: usize) {
    for (i, &(s, e)) in bounds.iter().enumerate() {
        let lo = a.max(s);
        let hi = b.min(e);
        if lo < hi {
            out[i].push_str(&full[lo..hi]);
        }
    }
}

/// XML for one slot after its text changed.
fn emit_slot(xml: &str, fmt: Fmt, s: &Slot, new: &str) -> String {
    if new == s.text {
        return xml[s.span.0..s.span.1].to_string();
    }
    if new.is_empty() {
        return match (fmt, s.kind) {
            (Fmt::Odf, _) | (_, SlotKind::Break) | (_, SlotKind::Tab) => String::new(),
            _ => format!("<{0}:t{1}></{0}:t>", fmt.ns(), if fmt.preserve() { " xml:space=\"preserve\"" } else { "" }),
        };
    }
    match fmt {
        Fmt::Odf => emit_odf(new),
        _ => emit_ooxml(new, fmt.ns(), fmt.preserve()),
    }
}

/// OOXML run content: newlines and tabs become `<w:br/>` / `<w:tab/>` elements.
fn emit_ooxml(text: &str, ns: &str, preserve: bool) -> String {
    let mut out = String::new();
    let mut buf = String::new();
    for ch in text.chars() {
        match ch {
            '\r' => {}
            '\n' => {
                flush_t(&mut buf, &mut out, ns, preserve);
                out.push_str(&format!("<{ns}:br/>"));
            }
            '\t' => {
                flush_t(&mut buf, &mut out, ns, preserve);
                out.push_str(&format!("<{ns}:tab/>"));
            }
            _ => buf.push(ch),
        }
    }
    flush_t(&mut buf, &mut out, ns, preserve);
    out
}

fn flush_t(buf: &mut String, out: &mut String, ns: &str, preserve: bool) {
    if buf.is_empty() {
        return;
    }
    let sp = if preserve { " xml:space=\"preserve\"" } else { "" };
    let e = xml_escape(buf);
    out.push_str(&format!("<{ns}:t{sp}>{e}</{ns}:t>", e = e));
    buf.clear();
}

/// ODF content: `<text:line-break/>`, `<text:tab/>` and `<text:s text:c="N"/>`.
fn emit_odf(text: &str) -> String {
    let mut out = String::new();
    let mut buf = String::new();
    let mut sp = 0usize;
    for ch in text.chars() {
        match ch {
            '\r' => {}
            '\n' => {
                odf_buf(&mut buf, &mut out);
                odf_sp(&mut sp, &mut out);
                out.push_str("<text:line-break/>");
            }
            '\t' => {
                odf_buf(&mut buf, &mut out);
                odf_sp(&mut sp, &mut out);
                out.push_str("<text:tab/>");
            }
            ' ' => {
                odf_buf(&mut buf, &mut out);
                sp += 1;
            }
            _ => buf.push(ch),
        }
    }
    odf_buf(&mut buf, &mut out);
    odf_sp(&mut sp, &mut out);
    out
}

fn odf_buf(buf: &mut String, out: &mut String) {
    if !buf.is_empty() {
        out.push_str(&xml_escape(buf));
        buf.clear();
    }
}

fn odf_sp(sp: &mut usize, out: &mut String) {
    if *sp == 0 {
        return;
    }
    if *sp == 1 {
        out.push(' ');
    } else {
        out.push_str(&format!("<text:s text:c=\"{sp}\"/>"));
    }
    *sp = 0;
}

/// Byte span of the first element named `name` that starts inside `[from, to)`.
fn find_child(tags: &[Tag], xml: &str, from: usize, to: usize, name: &str) -> Option<(usize, usize)> {
    for (i, t) in tags.iter().enumerate() {
        if t.span.1 <= from || t.span.0 >= to {
            continue;
        }
        if tag_name(xml, t) != name || t.close {
            continue;
        }
        if t.selfc {
            return Some(t.span);
        }
        let mut depth = 1usize;
        let mut j = i + 1;
        while j < tags.len() {
            if tag_name(xml, &tags[j]) == name {
                if tags[j].close {
                    depth -= 1;
                    if depth == 0 {
                        return Some((t.span.0, tags[j].span.1));
                    }
                } else if !tags[j].selfc {
                    depth += 1;
                }
            }
            j += 1;
        }
        return Some(t.span);
    }
    None
}

/// Length in bytes of the opening tag that starts at `start`.
fn open_tag_len(xml: &str, start: usize) -> usize {
    let mut i = start;
    let mut q: Option<char> = None;
    for ch in xml[start..].chars() {
        i += ch.len_utf8();
        match q {
            Some(c) => {
                if ch == c {
                    q = None;
                }
            }
            None => match ch {
                '"' | '\'' => q = Some(ch),
                '>' => break,
                _ => {}
            },
        }
    }
    i - start
}

/// A new paragraph that copies the properties of paragraph `idx` and holds `text`.
fn clone_para(xml: &str, flow: &Flow, fmt: Fmt, idx: usize, text: &str) -> String {
    let line = &flow.lines[idx];
    let mut open = xml[line.open.0..line.open.1].to_string();
    if let Some(p) = open.rfind("/>") {
        open.replace_range(p..p + 2, ">");
    }
    if fmt == Fmt::Odf {
        let mut s = open;
        s.push_str(&emit_odf(text));
        s.push_str("</");
        s.push_str(&line.close_name);
        s.push('>');
        return s;
    }
    let ns = fmt.ns();
    let from = line.open.0 + open_tag_len(xml, line.open.0);
    let mut ppr = find_child(&flow.tags, xml, from, line.span.1, fmt.ppr())
        .map(|s| xml[s.0..s.1].to_string())
        .unwrap_or_default();
    if ppr.contains("sectPr") {
        ppr.clear(); // a copied section break would reflow the document
    }
    let mut body = String::new();
    if let Some(rs) = find_child(&flow.tags, xml, from, line.span.1, fmt.run()) {
        let ro = open_tag_len(xml, rs.0);
        body.push_str(&xml[rs.0..rs.0 + ro]);
        if let Some(ws) = find_child(&flow.tags, xml, rs.0 + ro, rs.1, fmt.rpr()) {
            body.push_str(&xml[ws.0..ws.1]);
        }
    } else {
        body.push_str(&format!("<{ns}:r>"));
    }
    body.push_str(&emit_ooxml(text, ns, fmt.preserve()));
    body.push_str(&format!("</{ns}:r>"));
    let mut s = open;
    s.push_str(&ppr);
    s.push_str(&body);
    s.push_str("</");
    s.push_str(&line.close_name);
    s.push('>');
    s
}

// ───────────── driver ─────────────

/// Line and offset of a position in the flow text. A position that falls on a paragraph
/// separator belongs to the end of the previous line.
fn locate(starts: &[usize], flow: &Flow, a: usize, p: usize) -> (usize, usize) {
    let i = match starts.binary_search(&p) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let li = a + i;
    (li, (p - starts[i]).min(flow.lines[li].text.len()))
}

/// Write-back edits of one uninterrupted flow for one rule.
fn segment_edits(xml: &str, flow: &Flow, fmt: Fmt, a: usize, b: usize, r: &Rule, edits: &mut Vec<Edit>) -> usize {
    let mut text = String::new();
    let mut starts: Vec<usize> = Vec::with_capacity(b - a);
    for i in a..b {
        starts.push(text.len());
        text.push_str(&flow.lines[i].text);
        if i + 1 < b {
            text.push('\n');
        }
    }
    let caps: Vec<Captures> = r.re.captures_iter(&text).filter(|m| !m[0].is_empty()).collect();
    if caps.is_empty() {
        return 0;
    }
    let mut ops: BTreeMap<usize, Vec<(usize, usize, String)>> = BTreeMap::new();
    let mut dels: Vec<usize> = Vec::new();
    let mut ins: Vec<(usize, Vec<String>)> = Vec::new();
    for m in &caps {
        let w = m.get(0).unwrap();
        let (la, oa) = locate(&starts, flow, a, w.start());
        let (lb, ob) = locate(&starts, flow, a, w.end());
        let lens: Vec<usize> = (la..=lb).map(|i| flow.lines[i].text.len()).collect();
        let suffix = flow.lines[lb].text[ob..].to_string();
        let mut p = plan_match(la, oa, lb, ob, &lens, &r.expand(m), &suffix);
        for (k, v) in p.ops {
            ops.entry(k).or_default().extend(v);
        }
        dels.append(&mut p.dels);
        ins.append(&mut p.ins);
    }
    // a line that another match also writes into is emptied, not deleted
    let dl: HashSet<usize> = dels.into_iter().collect();
    let mut removed: Vec<usize> = Vec::new();
    for d in &dl {
        if let Some(list) = ops.get_mut(d) {
            let joined: String = list.iter().map(|(_, _, s)| s.as_str()).collect();
            *list = vec![(0, flow.lines[*d].text.len(), joined)];
        } else if flow.lines[*d].deletable {
            removed.push(*d);
        } else if !flow.lines[*d].text.is_empty() {
            ops.insert(*d, vec![(0, flow.lines[*d].text.len(), String::new())]);
        }
    }
    removed.sort_unstable();
    for d in removed {
        let l = &flow.lines[d];
        edits.push(Edit { start: l.span.0, end: l.span.1, text: String::new() });
    }
    for (li, list) in &ops {
        let line = &flow.lines[*li];
        if line.slots.is_empty() {
            let joined: String = list.iter().map(|(_, _, s)| s.as_str()).collect();
            if joined.is_empty() {
                continue;
            }
            let para = clone_para(xml, flow, fmt, *li, &joined);
            edits.push(Edit { start: line.span.0, end: line.span.1, text: para });
            continue;
        }
        let new = splice_line(line, list);
        for (si, s) in line.slots[line.usable.0..line.usable.1].iter().enumerate() {
            let nt = &new[si];
            if *nt == s.text {
                continue;
            }
            edits.push(Edit { start: s.span.0, end: s.span.1, text: emit_slot(xml, fmt, s, nt) });
        }
    }
    for (anchor, extras) in &ins {
        let line = &flow.lines[*anchor];
        let src = if line.slots.is_empty() && *anchor > a { *anchor - 1 } else { *anchor };
        let mut s = String::new();
        for t in extras {
            s.push_str(&clone_para(xml, flow, fmt, src, t));
        }
        edits.push(Edit { start: line.span.1, end: line.span.1, text: s });
    }
    caps.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Cursor;
    use std::io::Read;
    use std::path::Path;
    use zip::ZipArchive;

    fn rule(find: &str, repl: &str) -> Rule<'static> {
        let pat: String = find.split('\n').map(|s| regex::escape(s)).collect::<Vec<_>>().join("\n");
        Rule { re: Box::leak(Box::new(Regex::new(&pat).unwrap())), repl: Box::leak(repl.to_string().into_boxed_str()), is_regex: true, edge_blanks: false }
    }

    #[test]
    fn word_aligned_multiline_preserves_formatting() {
        let xml = "<w:p><w:pPr><w:jc w:val=\"center\"/></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t>A</w:t></w:r></w:p><w:p><w:r><w:rPr><w:i/></w:rPr><w:t>B</w:t></w:r></w:p>";
        let (out, n) = replace(xml, Fmt::Word, &[rule("A\nB", "X\nY")]);
        assert_eq!(n, 1);
        assert!(out.contains("<w:jc w:val=\"center\"/>"));
        assert!(out.contains("<w:b/>"));
        assert!(out.contains("<w:i/>"));
        assert!(out.contains(">X<"));
        assert!(out.contains(">Y<"));
    }

    #[test]
    fn word_fewer_lines_delete_paragraphs() {
        let xml = "<w:p><w:t>A</w:t></w:p><w:p><w:t>B</w:t></w:p><w:p><w:t>C</w:t></w:p>";
        let (out, n) = replace(xml, Fmt::Word, &[rule("A\nB\nC", "X")]);
        assert_eq!(n, 1);
        assert_eq!(out.matches("<w:p").count(), 1);
        assert!(out.contains(">X<"));
    }

    #[test]
    fn word_more_lines_insert_paragraphs() {
        let xml = "<w:p><w:pPr/><w:r><w:rPr><w:b/></w:rPr><w:t>A</w:t></w:r></w:p><w:p><w:pPr/><w:r><w:rPr><w:i/></w:rPr><w:t>B</w:t></w:r></w:p>";
        let (out, n) = replace(xml, Fmt::Word, &[rule("A\nB", "1\n2\n3")]);
        assert_eq!(n, 1);
        let np = Regex::new(r"<w:p\b").unwrap().find_iter(&out).count();
        assert_eq!(np, 3);
        let first = out.find(">1<").unwrap();
        let second = out.find(">2<").unwrap();
        let third = out.find(">3<").unwrap();
        assert!(first < second && second < third);
        assert_eq!(out.matches("<w:b/>").count(), 1);
        assert_eq!(out.matches("<w:i/>").count(), 2);
    }

    #[test]
    fn word_partial_edges_keep_prefix_suffix() {
        let xml = "<w:p><w:t>preA</w:t></w:p><w:p><w:t>Bpost</w:t></w:p>";
        let (out, _) = replace(xml, Fmt::Word, &[rule("A\nB", "X\nY")]);
        assert!(out.contains(">preX<"));
        assert!(out.contains(">Ypost<"));
    }

    #[test]
    fn word_match_does_not_cross_table_cell() {
        let xml = "<w:tc><w:p><w:t>A</w:t></w:p></w:tc><w:tc><w:p><w:t>B</w:t></w:p></w:tc>";
        let (out, n) = replace(xml, Fmt::Word, &[rule("A\nB", "X")]);
        assert_eq!(n, 0);
        assert_eq!(out, xml);
    }

    #[test]
    fn word_non_deletable_paragraph_is_emptied() {
        let xml = "<w:p><w:t>A</w:t></w:p><w:p><w:drawing/><w:t>B</w:t></w:p>";
        let (out, n) = replace(xml, Fmt::Word, &[rule("A\nB", "X")]);
        assert_eq!(n, 1);
        assert!(out.contains("<w:drawing/>"));
        assert!(out.contains(">X<"));
        assert!(!out.contains(">B<"));
    }

    #[test]
    fn word_soft_break_is_matchable() {
        let xml = "<w:p><w:t>A</w:t><w:br/><w:t>B</w:t></w:p>";
        let (out, n) = replace(xml, Fmt::Word, &[rule("A\nB", "X")]);
        assert_eq!(n, 1);
        assert!(!out.contains("<w:br/>"));
        assert!(out.contains(">X<"));
    }

    #[test]
    fn pptx_aligned_multiline() {
        let xml = "<a:p><a:r><a:t>A</a:t></a:r></a:p><a:p><a:r><a:t>B</a:t></a:r></a:p>";
        let (out, n) = replace(xml, Fmt::Slides, &[rule("A\nB", "X\nY")]);
        assert_eq!(n, 1);
        assert!(out.contains(">X<") && out.contains(">Y<"));
        assert!(!out.contains("xml:space"));
    }

    #[test]
    fn odf_aligned_multiline() {
        let xml = "<text:p>A</text:p><text:p>B</text:p>";
        let (out, n) = replace(xml, Fmt::Odf, &[rule("A\nB", "X\nY")]);
        assert_eq!(n, 1);
        assert_eq!(out, "<text:p>X</text:p><text:p>Y</text:p>");
    }

    #[test]
    fn odf_fewer_lines_delete_paragraphs() {
        let xml = "<text:p>A</text:p><text:p>B</text:p><text:p>C</text:p>";
        let (out, n) = replace(xml, Fmt::Odf, &[rule("A\nB\nC", "X")]);
        assert_eq!(n, 1);
        assert_eq!(out, "<text:p>X</text:p>");
    }

    #[test]
    fn odf_more_lines_insert_paragraphs() {
        let xml = "<text:p>A</text:p><text:p>B</text:p>";
        let (out, n) = replace(xml, Fmt::Odf, &[rule("A\nB", "1\n2\n3")]);
        assert_eq!(n, 1);
        assert_eq!(out, "<text:p>1</text:p><text:p>2</text:p><text:p>3</text:p>");
    }

    #[test]
    fn word_ignore_ws_crosses_paragraphs() {
        let xml = "<w:p><w:t>A</w:t></w:p><w:p><w:t>B</w:t></w:p>";
        let re = Regex::new(r"A\s+B").unwrap();
        let rule = Rule { re: &re, repl: "X", is_regex: true, edge_blanks: false };
        let (out, n) = replace(xml, Fmt::Word, &[rule]);
        assert_eq!(n, 1);
        assert_eq!(out.matches("<w:p").count(), 1);
        assert!(out.contains(">X<"));
    }

    #[test]
    fn real_docx_multiline_blocks_replaced() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("3. Контур.docx");
        let Ok(data) = fs::read(&path) else { return }; // untracked fixture; absent on CI
        let Ok(mut ar) = ZipArchive::new(Cursor::new(data)) else { return };
        let mut doc = None;
        for i in 0..ar.len() {
            if let Ok(mut f) = ar.by_index(i) {
                if f.name() == "word/document.xml" {
                    let mut s = String::new();
                    if f.read_to_string(&mut s).is_ok() {
                        doc = Some(s);
                    }
                }
            }
        }
        let Some(doc) = doc else { return };
        let rp = Regex::new(r"(?s)<w:p(?:\s[^>]*)?>.*?</w:p>").unwrap();
        let rt = Regex::new(r"<w:t[^>]*>(.*?)</w:t>").unwrap();
        let paras: Vec<String> = rp.captures_iter(&doc).map(|c| {
            rt.captures_iter(&c[0]).filter_map(|m| m.get(1).map(|g| g.as_str())).collect()
        }).collect();
        assert!(paras.len() >= 15);
        let b1 = &paras[2..=7];
        assert_eq!(b1[0], "443035,г. Самара, ул. Победа 132 А каб №202");
        assert_eq!(b1[1], "Testing Laboratory");
        let b2 = &paras[9..=14];
        assert_eq!(b2[0], "Подрядчик: ООО «ПЭБ».");
        let r1 = rule(&b1.join("\n"), "заменено\nстрока\nдругая\neще одна\nпятая\nшестая");
        let r2 = rule(&b2.join("\n"), "Подрядчик: ООО \"Новый\".\nЗаказчик: ООО \"Новый Заказчик\".\nАдрес: город Новый.\nМесторождение: новое.\nОбъект: новое оборудование.\nДата испытания: от 1 января 2020г.");
        let (out, n) = replace(&doc, Fmt::Word, &[r1, r2]);
        assert_eq!(n, 2);
        assert_eq!(out.matches("Testing Laboratory").count(), 1); // flattened single-line paragraph 1 remains
        assert!(!out.contains("Сибирская Сервисная"));
        assert!(out.contains("заменено"));
        assert!(out.contains("\"Новый\""));
    }
}

/// One pass of the rules over a part; matches may span paragraphs.
pub fn replace(xml: &str, fmt: Fmt, rules: &[Rule]) -> (String, usize) {
    let mut cur = xml.to_string();
    let mut total = 0usize;
    for r in rules {
        let flow = parse_flow(&cur, fmt, r.edge_blanks);
        let mut edits: Vec<Edit> = Vec::new();
        let mut n = 0usize;
        for &(a, b) in &flow.segs {
            n += segment_edits(&cur, &flow, fmt, a, b, r, &mut edits);
        }
        if n == 0 {
            continue;
        }
        cur = apply_edits(&cur, edits);
        total += n;
    }
    (cur, total)
}
