(() => {
  "use strict";
  const T = window.__TAURI__;
  const invoke = T.core.invoke;
  const $ = (id) => document.getElementById(id);

  // key in settings, i18n key, default value
  const OPTS = [
    ["recursive", "opt.recursive", true],
    ["caseSensitive", "opt.caseSensitive", false],
    ["ignoreWs", "opt.ignoreWs", false],
    ["regex", "opt.regex", false],
    ["backup", "opt.backup", true],
    ["binary", "opt.binary", false],
    ["convertLegacy", "opt.convertLegacy", true],
  ];

  // ───────────────────────── i18n ─────────────────────────
  // Bundled languages: src/locales/<code>.json listed in src/locales/index.json.
  // User languages: <app config dir>/locales/*.json (loaded through the backend).
  const FALLBACK = "en";
  const SEP = "\u001f"; // separator used by the backend in "code<SEP>arg<SEP>arg" messages
  const dicts = {};     // code -> { name, strings }
  let lang = FALLBACK;

  function lookup(code, key) {
    const d = dicts[code];
    return d && d.strings[key] !== undefined ? d.strings[key] : undefined;
  }
  function t(key, ...args) {
    let s = lookup(lang, key);
    if (s === undefined) s = lookup(FALLBACK, key);
    if (s === undefined) s = key;
    return s.replace(/\{(\d+)\}/g, (m, i) => (args[i] !== undefined ? args[i] : m));
  }
  // translate a message coming from the backend ("skip.binary", "err.empty_find\u001f3", or raw text)
  function tb(msg) {
    const [code, ...args] = String(msg).split(SEP);
    const known = lookup(lang, code) !== undefined || lookup(FALLBACK, code) !== undefined;
    return known ? t(code, ...args) : String(msg).split(SEP).join(" ");
  }
  function register(json, hintCode) {
    if (!json || typeof json !== "object") return;
    const meta = json._meta || {};
    const code = String(meta.code || hintCode || "").trim();
    if (!code) return;
    const strings = {};
    for (const [k, v] of Object.entries(json)) if (k !== "_meta" && typeof v === "string") strings[k] = v;
    dicts[code] = { name: String(meta.name || code), strings };
  }
  async function loadBundled() {
    try {
      const index = await (await fetch("locales/index.json")).json();
      for (const code of index) {
        try { register(await (await fetch(`locales/${code}.json`)).json(), code); }
        catch (e) { console.warn("locale failed:", code, e); }
      }
    } catch (e) { console.warn("locales/index.json failed", e); }
  }
  async function loadUser() {
    try {
      for (const raw of await invoke("user_locales")) {
        try { register(JSON.parse(raw)); } catch (e) { console.warn("bad user locale", e); }
      }
    } catch { /* backend without the command: ignore */ }
  }
  function populateLangs() {
    const sel = $("lang");
    const codes = Object.keys(dicts).sort((a, b) => dicts[a].name.localeCompare(dicts[b].name));
    sel.textContent = "";
    for (const code of codes) {
      const o = document.createElement("option");
      o.value = code; o.textContent = dicts[code].name;
      sel.appendChild(o);
    }
    sel.value = dicts[lang] ? lang : FALLBACK;
  }
  function detectLang(saved) {
    if (saved && dicts[saved]) return saved;
    for (const l of navigator.languages || [navigator.language || FALLBACK]) {
      const full = l.toLowerCase(), short = full.split("-")[0];
      if (dicts[full]) return full;
      if (dicts[short]) return short;
    }
    return FALLBACK;
  }
  function applyStatic() {
    document.documentElement.lang = lang;
    document.querySelectorAll("[data-i18n]").forEach((el) => { el.textContent = t(el.dataset.i18n); });
    document.querySelectorAll("[data-i18n-ph]").forEach((el) => { el.placeholder = t(el.dataset.i18nPh); });
    document.querySelectorAll("[data-i18n-title]").forEach((el) => { el.title = t(el.dataset.i18nTitle); });
  }

  // ───────────────────────── state ─────────────────────────
  const rules = [];
  let running = false;
  let lastDry = false;
  let lastFolder = "";
  let lastStats = null;
  let statusState = null;      // { key, args }
  let emptyKey = "results.empty";
  let pending = [];
  let flushScheduled = false;

  // ───────────────────────── settings ─────────────────────────
  const KEY = "text-replacer-settings";
  function loadSettings() {
    try { return JSON.parse(localStorage.getItem(KEY)) || {}; } catch { return {}; }
  }
  function saveSettings() {
    const s = { folder: $("folder").value, mask: $("mask").value, maxsize: $("maxsize").value, theme: $("theme").value, lang };
    for (const [k] of OPTS) s["o_" + k] = $("opt-" + k).checked;
    try { localStorage.setItem(KEY, JSON.stringify(s)); } catch { /* ignore */ }
  }

  // ───────────────────────── switches ─────────────────────────
  for (const [key, i18n, def] of OPTS) {
    const label = document.createElement("label");
    label.className = "switch";
    label.innerHTML = '<input type="checkbox"><span class="tr"></span><span class="tx"></span>';
    label.querySelector("input").id = "opt-" + key;
    label.querySelector("input").checked = def;
    label.querySelector(".tx").dataset.i18n = i18n;
    $("switches").appendChild(label);
  }

  const st = loadSettings();
  if (st.folder) $("folder").value = st.folder;
  if (st.mask) $("mask").value = st.mask;
  if (st.maxsize !== undefined) $("maxsize").value = st.maxsize;
  for (const [k] of OPTS) if (("o_" + k) in st) $("opt-" + k).checked = st["o_" + k];
  $("theme").value = st.theme || "system";

  function applyTheme() {
    const v = $("theme").value;
    if (v === "system") document.documentElement.removeAttribute("data-theme");
    else document.documentElement.setAttribute("data-theme", v);
  }
  applyTheme();
  document.addEventListener("change", () => { applyTheme(); saveSettings(); });
  document.addEventListener("input", saveSettings);

  // ───────────────────────── dynamic texts ─────────────────────────
  const notify = (text, kind = "warning") => T.dialog.message(text, { title: t("dlg.title"), kind });

  function setStatus(key, ...args) { statusState = key ? { key, args } : null; renderStatus(); }
  function renderStatus() { $("status").textContent = statusState ? t(statusState.key, ...statusState.args) : ""; }
  function setEmpty(key) { emptyKey = key; renderEmpty(); }
  function renderEmpty() { $("empty").textContent = t(emptyKey); }

  function short(s, n = 70) {
    s = s.replace(/\r/g, "").replace(/\n/g, "↵");
    return s.length > n ? s.slice(0, n - 1) + "…" : s;
  }
  function renderRules() {
    const box = $("rules");
    box.textContent = "";
    if (!rules.length) {
      const d = document.createElement("div");
      d.className = "none";
      d.textContent = t("rules.empty");
      box.appendChild(d);
      return;
    }
    rules.forEach((r, i) => {
      const row = document.createElement("div");
      row.className = "rule";
      const n = document.createElement("span"); n.className = "n"; n.textContent = i + 1 + ".";
      const a = document.createElement("span"); a.className = "t"; a.textContent = short(r.find);
      const ar = document.createElement("span"); ar.className = "arrow"; ar.textContent = "→";
      const b = document.createElement("span"); b.className = "t"; b.textContent = short(r.repl);
      const x = document.createElement("button"); x.textContent = "✕"; x.title = t("rule.delete");
      x.onclick = () => { rules.splice(i, 1); renderRules(); };
      row.append(n, a, ar, b, x);
      box.appendChild(row);
    });
  }

  function statusText(status, info, dry) {
    if (status === "ok") return t(dry ? "row.found" : "row.replaced") + (!dry && info ? " → " + info : "");
    if (status === "skipped") return t("row.skipped", tb(info));
    return t("row.error", tb(info));
  }

  function renderStats() {
    const box = $("stats");
    box.textContent = "";
    if (!lastStats) return;
    const s = lastStats;
    const items = [
      ["stat.files", s.files], ["stat.changed", s.changed], ["stat.replaced", s.replaced],
      ["stat.skipped", s.skipped], ["stat.errors", s.errors],
    ];
    for (const [key, val] of items) {
      const d = document.createElement("div");
      d.className = "stat" + (key === "stat.errors" && val ? " err" : "");
      const b = document.createElement("b"); b.textContent = String(val);
      const sp = document.createElement("span"); sp.textContent = t(key);
      d.append(b, sp);
      box.appendChild(d);
    }
  }

  function rerender() {
    applyStatic();
    renderRules();
    renderStatus();
    renderEmpty();
    renderStats();
    document.querySelectorAll("#rows tr").forEach((tr) => {
      tr.lastChild.textContent = statusText(tr.dataset.status, tr.dataset.info, tr.dataset.dry === "1");
    });
  }

  // ───────────────────────── rules / dialogs ─────────────────────────
  $("addRule").onclick = () => {
    const f = $("find").value;
    if (!f) return notify(t("dlg.empty_find"));
    rules.push({ find: f, repl: $("repl").value });
    $("find").value = ""; $("repl").value = "";
    renderRules();
  };

  $("loadRules").onclick = async () => {
    try {
      const p = await T.dialog.open({ multiple: false, directory: false, title: t("dlg.rules_dialog") });
      if (typeof p !== "string") return;
      rules.push(...(await invoke("load_rules", { path: p, regex: $("opt-regex").checked })));
      renderRules();
    } catch (e) { notify(tb(e), "error"); }
  };

  $("browse").onclick = async () => {
    try {
      const p = await T.dialog.open({ directory: true, multiple: false, defaultPath: $("folder").value || undefined });
      if (typeof p === "string") { $("folder").value = p; saveSettings(); }
    } catch (e) { notify(tb(e), "error"); }
  };

  $("lang").onchange = () => { lang = $("lang").value; saveSettings(); rerender(); };

  $("openLoc").onclick = async () => {
    try {
      const file = await invoke("prepare_locales_dir");
      await T.opener.revealItemInDir(file);
    } catch (e) { notify(tb(e), "error"); }
  };

  // pick up translations dropped into the user folder while the app is open
  window.addEventListener("focus", async () => {
    const before = Object.keys(dicts).join();
    await loadUser();
    if (Object.keys(dicts).join() !== before) populateLangs();
  });

  // ───────────────────────── drag & drop ─────────────────────────
  try {
    T.webview.getCurrentWebview().onDragDropEvent(async (ev) => {
      const p = ev.payload;
      if (p.type === "enter" || p.type === "over") $("drop").classList.remove("hidden");
      else if (p.type === "leave") $("drop").classList.add("hidden");
      else if (p.type === "drop") {
        $("drop").classList.add("hidden");
        if (p.paths && p.paths.length) {
          const info = await invoke("inspect_path", { path: p.paths[0] });
          $("folder").value = info.dir;
          if (info.file) $("mask").value = info.file;
          saveSettings();
        }
      }
    });
  } catch (e) { console.warn("drag-and-drop unavailable", e); }

  // ───────────────────────── results ─────────────────────────
  function setBusy(b) {
    running = b;
    $("run").disabled = b; $("dry").disabled = b;
    $("spinner").classList.toggle("hidden", !b);
  }

  function relPath(p) {
    if (lastFolder && p.startsWith(lastFolder)) return p.slice(lastFolder.length).replace(/^[\\/]+/, "") || p;
    return p;
  }

  function flush() {
    flushScheduled = false;
    if (!pending.length) return;
    const frag = document.createDocumentFragment();
    for (const e of pending) {
      const tr = document.createElement("tr");
      tr.dataset.path = e.path; tr.dataset.status = e.status;
      tr.dataset.info = e.info; tr.dataset.dry = lastDry ? "1" : "0";
      const td1 = document.createElement("td"); td1.className = "path"; td1.title = e.path;
      const bdi = document.createElement("bdi"); bdi.textContent = relPath(e.path); td1.appendChild(bdi);
      const td2 = document.createElement("td"); td2.className = "num"; td2.textContent = e.count ? String(e.count) : "—";
      const td3 = document.createElement("td");
      td3.className = "s-" + e.status;
      td3.textContent = statusText(e.status, e.info, lastDry);
      tr.append(td1, td2, td3);
      frag.appendChild(tr);
    }
    pending = [];
    $("rows").appendChild(frag);
    $("empty").classList.add("hidden");
  }

  T.event.listen("file-result", (e) => {
    pending.push(e.payload);
    if (!flushScheduled) { flushScheduled = true; requestAnimationFrame(flush); }
  });

  $("rows").addEventListener("dblclick", (ev) => {
    const tr = ev.target.closest("tr");
    if (tr && tr.dataset.path) T.opener.revealItemInDir(tr.dataset.path).catch(() => {});
  });

  // ───────────────────────── run ─────────────────────────
  async function start(dry) {
    if (running) return;
    const work = rules.slice();
    const f = $("find").value;
    if (f) work.push({ find: f, repl: $("repl").value });
    if (!work.length) return notify(t("dlg.nothing"));
    const folder = $("folder").value.trim();
    if (!folder) return notify(t("dlg.no_folder"));

    if (!dry) {
      const ok = await T.dialog.ask(t("dlg.confirm.text", work.length, folder), {
        title: t("dlg.confirm.title"), kind: "warning", okLabel: t("dlg.yes"), cancelLabel: t("dlg.no"),
      });
      if (!ok) return;
    }

    const req = {
      folder, mask: $("mask").value || "*", rules: work,
      recursive: $("opt-recursive").checked, caseSensitive: $("opt-caseSensitive").checked,
      ignoreWs: $("opt-ignoreWs").checked, regex: $("opt-regex").checked,
      backup: $("opt-backup").checked, binary: $("opt-binary").checked,
      dry, maxSizeMb: Math.max(0, Math.floor(Number($("maxsize").value) || 0)),
      convertLegacy: $("opt-convertLegacy").checked,
    };

    lastDry = dry; lastFolder = folder; lastStats = null; pending = [];
    $("rows").textContent = "";
    renderStats();
    setEmpty("results.searching");
    $("empty").classList.remove("hidden");
    setStatus("status.working");
    setBusy(true);
    const t0 = performance.now();
    try {
      const s = await invoke("run_replace", { req });
      flush();
      lastStats = s;
      renderStats();
      const sec = ((performance.now() - t0) / 1000).toFixed(1);
      setStatus(dry ? "status.preview_done" : "status.done", sec);
      if (!s.changed && !s.errors && !s.skipped) setEmpty("results.none");
      else $("empty").classList.add("hidden");
    } catch (e) {
      setStatus("status.error");
      setEmpty("results.empty");
      notify(tb(e), "error");
    } finally {
      setBusy(false);
    }
  }
  $("dry").onclick = () => start(true);
  $("run").onclick = () => start(false);

  // ───────────────────────── init ─────────────────────────
  (async () => {
    await loadBundled();
    await loadUser();
    lang = detectLang(st.lang);
    populateLangs();
    rerender();
  })();
})();
