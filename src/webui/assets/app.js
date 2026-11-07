// InstaFind main window (src/webui). Talks to the Rust side only through
// window.ipc.postMessage (requests) and window.__instafind.recv (answers);
// see src/webui/bridge.rs for the message types. File names and paths are
// untrusted: they only ever reach the page through textContent.
"use strict";
(() => {
  const TOKEN = document.querySelector('meta[name="instafind-token"]').content;
  const LANG = document.documentElement.lang === "en" ? "en" : "ar";
  const FIRST_PAGE_GUARD = 300; // px from the bottom that asks for more rows

  // ---------- strings ----------
  const T = {
    ar: {
      ph: "ابحث باسم الملف…", type: "النوع", sort: "الأنسب أولاً",
      cName: "الاسم والمسار", cSize: "الحجم", cDate: "آخر تعديل",
      results: "النتائج", preview: "المعاينة",
      recent: "فتحتها مؤخراً", recentNone: "الملفات التي تفتحها تظهر هنا.", tryThese: "جرّب هذه",
      trust: "كل شيء يبقى على جهازك. لا نجمع أي بيانات استخدام.",
      tips: [["تقرير .pdf", "ملفات PDF فقط"], ["حجم:>10م", "أكبر من 10 ميغابايت"], ["تاريخ:هذا_الأسبوع", "عُدّلت هذا الأسبوع"], ['"عقد إيجار"', "العبارة كما هي"], ["جذر: كتاب", "مكتبة، الكاتب، كتابات"], ["hgjrvdv", "لوحة المفاتيح الخطأ ← «التقرير»"]],
      kNav: "تنقل", kOpen: "فتح", kFolder: "المجلد",
      open: "فتح", openFolder: "فتح المجلد", copyPath: "نسخ المسار",
      fType: "النوع", fSize: "الحجم", fMod: "آخر تعديل", folder: "مجلد", file: "ملف",
      nres: (n) => (n === 0 ? "لا نتائج" : n === 1 ? "نتيجة واحدة" : n === 2 ? "نتيجتان" : n <= 10 ? `${num(n)} نتائج` : `${num(n)} نتيجة`),
      count: (n, ms) => `عدد النتائج: ${num(n)} (خلال ${num(ms)} ملّي ثانية)`,
      files: (n) => `الفهرس محدَّث · ${num(n)} ملف`,
      noresH: (q) => `لا توجد نتائج لـ «${q}»`,
      noresSub: (n, kind) => `بحثنا في أسماء ${num(n)} ملف في كل الأقراص${kind ? `، ضمن «${kind}» فقط` : ""}.`,
      noresIx: "الفهرسة ما زالت جارية، وقد يظهر الملف حين تنتهي.",
      tagHead: "لا شيء يضيع بصمت", tagTail: ": إذا لم نجد شيئاً نقول لك أين بحثنا وماذا تجرّب.",
      loading: "جارِ تحميل الفهرس…", ixSub: "يمكنك البحث الآن، وقد تكون النتائج ناقصة حتى ننتهي. القائمة تتحدث وحدها.",
      more: "مرّر لعرض المزيد",
      sugg: { query: "يبحث بهذا النص بدلاً مما كتبت", all_kinds: "يزيل فلتر النوع", web: "يفتح المتصفح، ولا يرسل شيئاً غير ما كتبته" },
      now: "الآن", minAgo: (n) => (n === 1 ? "قبل دقيقة" : n === 2 ? "قبل دقيقتين" : n <= 10 ? `قبل ${num(n)} دقائق` : `قبل ${num(n)} دقيقة`),
      today: "اليوم", yesterday: "أمس",
      units: ["بايت", "كيلوبايت", "ميغابايت", "غيغابايت", "تيرابايت"],
    },
    en: {
      ph: "Search file names…", type: "Type", sort: "Best match first",
      cName: "Name and path", cSize: "Size", cDate: "Modified",
      results: "Results", preview: "Preview",
      recent: "Recently opened", recentNone: "Files you open show up here.", tryThese: "Try these",
      trust: "Everything stays on your PC. No usage data is collected.",
      tips: [["report .pdf", "PDF files only"], ["size:>10mb", "Bigger than 10 MB"], ["dm:thisweek", "Changed this week"], ['"lease agreement"', "Exact phrase"], ["root: كتاب", "Arabic root: مكتبة، الكاتب"], ["احمد", "Also finds «أحمد»"]],
      kNav: "Move", kOpen: "Open", kFolder: "Folder",
      open: "Open", openFolder: "Open folder", copyPath: "Copy path",
      fType: "Type", fSize: "Size", fMod: "Modified", folder: "Folder", file: "File",
      nres: (n) => (n === 0 ? "No results" : n === 1 ? "1 result" : `${num(n)} results`),
      count: (n, ms) => `${num(n)} results in ${num(ms)} ms`,
      files: (n) => `Index up to date · ${num(n)} files`,
      noresH: (q) => `No results for “${q}”`,
      noresSub: (n, kind) => `We searched ${num(n)} file names on all drives${kind ? `, “${kind}” only` : ""}.`,
      noresIx: "Indexing is still running; the file may appear when it finishes.",
      tagHead: "Nothing gets lost silently", tagTail: ": when we find nothing, we tell you where we looked and what to try.",
      loading: "Loading the index…", ixSub: "You can search now; results may be incomplete until we finish. The list updates by itself.",
      more: "Scroll for more",
      sugg: { query: "Searches for this instead", all_kinds: "Removes the type filter", web: "Opens your browser with only what you typed" },
      now: "Just now", minAgo: (n) => (n === 1 ? "1 min ago" : `${num(n)} min ago`),
      today: "Today", yesterday: "Yesterday",
      units: ["bytes", "KB", "MB", "GB", "TB"],
    },
  }[LANG];

  // ---------- state ----------
  const S = {
    seq: 0, shown: -1, q: "", kind: 0, kindLabel: "", kinds: [], digits: false,
    rows: [], total: 0, ms: 0, done: true, sel: 0, asked: 0,
    files: 0, state: "ready", stateText: "",
    recent: [], sugg: [], ssel: 0, view: "empty", note: "", notice: "",
  };

  const $ = (id) => document.getElementById(id);
  const q = $("q"), list = $("list");

  // ---------- numbers, sizes, dates ----------
  const AR_DIGITS = "٠١٢٣٤٥٦٧٨٩";
  function digits(s) { return S.digits && LANG === "ar" ? String(s).replace(/[0-9]/g, (d) => AR_DIGITS[d]) : String(s); }
  function num(n) { return digits(Number(n).toLocaleString("en-US")); }
  function size(bytes) {
    let v = Number(bytes), u = 0;
    while (v >= 1024 && u < T.units.length - 1) { v /= 1024; u++; }
    const s = u === 0 ? String(v) : v >= 100 ? v.toFixed(0) : v >= 10 ? v.toFixed(0) : v.toFixed(1).replace(/\.0$/, "");
    return `${digits(s)} ${T.units[u]}`;
  }
  const LOCALE = LANG === "ar" ? "ar-u-nu-latn" : "en-US";
  const fmtDay = new Intl.DateTimeFormat(LOCALE, { day: "numeric", month: "long" });
  const fmtDayYear = new Intl.DateTimeFormat(LOCALE, { day: "numeric", month: "long", year: "numeric" });
  const fmtTime = new Intl.DateTimeFormat(LOCALE, { hour: "numeric", minute: "2-digit" });
  function when(secs) {
    if (!secs) return "";
    const d = new Date(secs * 1000), now = new Date();
    const mins = Math.floor((now - d) / 60000);
    if (mins >= 0 && mins < 1) return T.now;
    if (mins >= 0 && mins < 60) return digits(T.minAgo(mins));
    const day = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
    const days = Math.round((day(now) - day(d)) / 86400000);
    if (days === 0) return digits(`${T.today} ${fmtTime.format(d)}`);
    if (days === 1) return digits(`${T.yesterday} ${fmtTime.format(d)}`);
    return digits((d.getFullYear() === now.getFullYear() ? fmtDay : fmtDayYear).format(d));
  }

  // ---------- DOM helpers (no HTML from data, ever) ----------
  function h(tag, attrs, ...kids) {
    const el = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v === null || v === undefined || v === false) continue;
      if (k === "class") el.className = v; else el.setAttribute(k, v === true ? "" : v);
    }
    for (const k of kids) if (k !== null && k !== undefined) el.append(k);
    return el;
  }
  // Static icon markup only (constants below); never data.
  const SVG_NS = "http://www.w3.org/2000/svg";
  function icon(paths, cls = "i") {
    const svg = document.createElementNS(SVG_NS, "svg");
    svg.setAttribute("class", cls); svg.setAttribute("viewBox", "0 0 24 24"); svg.setAttribute("aria-hidden", "true");
    for (const [tag, a] of paths) {
      const el = document.createElementNS(SVG_NS, tag);
      for (const [k, v] of Object.entries(a)) el.setAttribute(k, v);
      svg.append(el);
    }
    return svg;
  }
  const I = {
    open: [["path", { d: "M14 4h6v6M20 4l-9 9" }], ["path", { d: "M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5" }]],
    folder: [["path", { d: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" }]],
    copy: [["rect", { x: "8", y: "8", width: "12", height: "12", rx: "2" }], ["path", { d: "M16 8V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h3" }]],
    bulb: [["path", { d: "M9 18h6M10 21h4M12 3a6 6 0 0 0-4 10.5c.7.7 1 1.5 1 2.5h6c0-1 .3-1.8 1-2.5A6 6 0 0 0 12 3z" }]],
    filter: [["path", { d: "M4 5h16l-6 8v6l-4-2v-4z" }]],
    globe: [["circle", { cx: "12", cy: "12", r: "9" }], ["path", { d: "M3 12h18M12 3c2.5 2.7 3.8 5.7 3.8 9s-1.3 6.3-3.8 9c-2.5-2.7-3.8-5.7-3.8-9S9.5 5.7 12 3z" }]],
    search: [["circle", { cx: "11", cy: "11", r: "7" }], ["path", { d: "m20 20-3.6-3.6" }]],
    x: [["path", { d: "M6 6l12 12M18 6 6 18" }]],
    down: [["path", { d: "m6 9 6 6 6-6" }]],
  };

  // File-type badge: colour by family, the extension as its label.
  const FAMILY = [
    ["#D93F3F", ["pdf"]], ["#1E8E4E", ["xls", "xlsx", "xlsm", "csv", "ods"]], ["#2F64D6", ["doc", "docx", "odt", "rtf"]],
    ["#D9682B", ["ppt", "pptx", "odp"]], ["#9A7A12", ["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "iso"]],
    ["#8A55C9", ["jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "tif", "tiff", "svg"]],
    ["#C2417F", ["mp3", "wav", "m4a", "aac", "flac", "ogg", "wma", "opus", "mp4", "mkv", "avi", "mov", "wmv", "webm"]],
    ["#123B2A", ["exe", "msi", "bat", "cmd", "ps1", "lnk", "appx", "msix"]],
  ];
  function badge(r, cls = "ft") {
    const svg = document.createElementNS(SVG_NS, "svg");
    svg.setAttribute("class", cls); svg.setAttribute("viewBox", "0 0 30 34"); svg.setAttribute("aria-hidden", "true");
    const path = (d, fill, extra) => { const p = document.createElementNS(SVG_NS, "path"); p.setAttribute("d", d); p.setAttribute("fill", fill); for (const [k, v] of Object.entries(extra || {})) p.setAttribute(k, v); svg.append(p); };
    if (r.folder) {
      path("M2 9a3 3 0 0 1 3-3h7l3 3h10a3 3 0 0 1 3 3v13a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3z", "#F2B544");
      path("M2 13h26v12a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3z", "#F7C95F");
      return svg;
    }
    const ext = String(r.ext || "").toLowerCase();
    const fam = FAMILY.find(([, exts]) => exts.includes(ext));
    path("M5 1h14l8 8v21a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3V4a3 3 0 0 1 3-3z", fam ? fam[0] : "#6B7F74");
    path("M19 1v5a3 3 0 0 0 3 3h5z", "#fff", { "fill-opacity": ".45" });
    const label = ext.toUpperCase().replace(/[^A-Z0-9]/g, "").slice(0, 4);
    if (label) {
      const t = document.createElementNS(SVG_NS, "text");
      for (const [k, v] of Object.entries({ x: "14.5", y: "25", "text-anchor": "middle", "font-size": label.length > 3 ? "6.2" : "7.5", "font-weight": "600", fill: "#fff" })) t.setAttribute(k, v);
      t.textContent = label;
      svg.append(t);
    }
    return svg;
  }

  // The name with the matched parts in <mark>; the name takes the direction of its first strong letter.
  function nameEl(r) {
    const el = h("bdi", { class: "t" });
    for (const [text, hl] of r.name) el.append(hl ? h("mark", null, text) : document.createTextNode(text));
    return el;
  }
  function plainName(r) { return r.name.map(([t]) => t).join(""); }

  // A full path, always left to right, each folder name isolated (<bdi>) so
  // Arabic names never swap places. Long paths are shortened in the middle.
  function pathEl(full, max = 70) {
    const unc = full.startsWith("\\\\");
    let segs = full.split("\\").filter(Boolean);
    if (full.length > max && segs.length > 3) {
      const keep = [segs[segs.length - 1]];
      let len = keep[0].length + segs[0].length + 4;
      for (let i = segs.length - 2; i > 0 && len + segs[i].length + 1 < max; i--) { keep.unshift(segs[i]); len += segs[i].length + 1; }
      segs = [segs[0], null, ...keep];
    }
    const el = h("span", { class: "path", dir: "ltr" });
    if (unc) el.append(h("span", { class: "sep" }, "\\\\"));
    segs.forEach((s, i) => {
      if (i) { el.append(h("span", { class: "sep" }, "\\"), h("wbr")); }
      el.append(s === null ? h("span", { class: "ell" }, "…") : h("bdi", null, s));
    });
    return el;
  }
  function joinPath(dir, name) { return dir.endsWith("\\") ? dir + name : dir + "\\" + name; }

  // ---------- talking to the program ----------
  function send(msg) {
    msg.t = TOKEN;
    try { window.ipc.postMessage(JSON.stringify(msg)); } catch (e) { /* the host is gone (closing) */ }
  }
  const handlers = {
    hello(m) {
      S.kinds = m.kinds || []; S.digits = !!m.digits;
      renderKind(); renderStatus();
    },
    results(m) {
      if (m.seq !== S.seq) return; // an older search: the user typed on
      // The same search again (the index changed): keep the selected file.
      const refresh = m.seq === S.shown && S.rows[S.sel];
      const was = refresh && joinPath(S.rows[S.sel].dir, plainName(S.rows[S.sel]));
      const keep = refresh ? m.rows.findIndex((r) => joinPath(r.dir, plainName(r)) === was) : -1;
      S.shown = m.seq;
      S.rows = m.rows; S.total = m.total; S.ms = m.ms; S.done = m.rows.length >= m.total;
      S.asked = 0; S.note = m.note || ""; S.sugg = m.suggestions || []; S.ssel = 0;
      S.files = m.files ?? S.files; S.kindLabel = m.kind_label || "";
      S.sel = keep > 0 ? keep : 0;
      render(!!refresh);
      announce();
    },
    rows(m) {
      if (m.seq !== S.seq || m.from !== S.rows.length) return;
      S.rows = S.rows.concat(m.rows); S.done = S.rows.length >= S.total; S.asked = 0;
      appendRows(m.from);
    },
    recent(m) { S.recent = m.rows || []; renderRecent(); },
    status(m) {
      S.state = m.state; S.stateText = m.text || ""; S.files = m.files;
      renderBanner(); renderStatus();
      if (S.view === "nores") renderNoResults();
    },
    query(m) { q.value = m.q; onInput(); },
    focus() { q.focus(); },
    notice(m) {
      S.notice = m.text; renderStatus();
      clearTimeout(handlers.noticeTimer);
      handlers.noticeTimer = setTimeout(() => { S.notice = ""; renderStatus(); }, 2500);
    },
  };
  Object.defineProperty(window, "__instafind", { value: Object.freeze({ recv: (m) => { const f = handlers[m && m.type]; if (f) f(m); } }) });

  // ---------- searching ----------
  function onInput() {
    S.q = q.value;
    S.seq++;
    if (!S.q.trim()) {
      S.rows = []; S.total = 0; S.sel = 0; S.view = "empty";
      render();
      return;
    }
    send({ op: "search", seq: S.seq, q: S.q, kind: S.kind });
  }
  function askMore() {
    if (S.done || S.asked || S.view !== "results") return;
    S.asked = S.rows.length;
    send({ op: "more", seq: S.seq, from: S.rows.length });
  }
  function act(op, row, listName = "results") {
    if (!row) return;
    send({ op, list: listName, seq: listName === "results" ? S.seq : 0, id: row.id });
  }

  // ---------- rendering ----------
  function setView(v) {
    S.view = v;
    $("view-results").hidden = v !== "results";
    $("view-empty").hidden = v !== "empty";
    $("view-nores").hidden = v !== "nores";
    q.setAttribute("aria-expanded", String(v === "results"));
    q.setAttribute("aria-controls", v === "nores" ? "sugg" : "list");
  }
  function render(refresh = false) {
    if (!S.q.trim()) setView("empty");
    else setView(S.rows.length ? "results" : "nores");
    $("meta").textContent = S.q.trim() ? T.nres(S.total) : "";
    if (S.view === "results") renderRows(refresh);
    if (S.view === "nores") renderNoResults();
    renderPreview();
    renderStatus();
    renderBanner();
  }
  function rowEl(r, i, listName) {
    const name = plainName(r);
    const el = h("div", { class: "row", role: "option", id: (listName === "recent" ? "rr" : "r") + i, "aria-selected": String(listName === "results" && i === S.sel), "aria-label": `${name}, ${r.dir}${r.folder ? "" : ", " + size(r.size)}, ${when(r.mtime)}` });
    el.dataset.i = i;
    el.append(
      h("span", null, badge(r)),
      h("span", { class: "nm" }, h("span", { class: "n1" }, nameEl(r)), h("span", { class: "n2" }, pathEl(r.dir))),
    );
    if (listName === "results") el.append(h("span", { class: "sz num" }, r.folder ? "" : size(r.size)));
    el.append(h("span", { class: "dt num" }, when(r.mtime)));
    return el;
  }
  function renderRows(refresh) {
    const top = list.scrollTop;
    list.replaceChildren(...S.rows.map((r, i) => rowEl(r, i, "results")));
    if (!S.done) list.append(h("div", { class: "more", "aria-hidden": "true" }, T.more));
    list.scrollTop = refresh ? top : 0;
    select(S.sel, refresh);
  }
  function appendRows(from) {
    const more = list.querySelector(".more");
    const frag = document.createDocumentFragment();
    S.rows.slice(from).forEach((r, k) => frag.append(rowEl(r, from + k, "results")));
    if (more) more.before(frag); else list.append(frag);
    if (S.done && more) more.remove();
  }
  function select(i, scroll = true) {
    if (!S.rows.length) { q.removeAttribute("aria-activedescendant"); return; }
    S.sel = Math.max(0, Math.min(S.rows.length - 1, i));
    for (const el of list.querySelectorAll('.row[aria-selected="true"]')) el.setAttribute("aria-selected", "false");
    const el = $("r" + S.sel);
    if (el) { el.setAttribute("aria-selected", "true"); if (scroll) el.scrollIntoView({ block: "nearest" }); }
    q.setAttribute("aria-activedescendant", "r" + S.sel);
    if (S.sel > S.rows.length - 20) askMore();
    renderPreview();
  }
  function renderPreview() {
    const pv = $("preview");
    const r = S.view === "results" ? S.rows[S.sel] : null;
    if (!r) { pv.replaceChildren(); return; }
    const kind = r.folder ? T.folder : r.ext ? `${r.ext.toUpperCase()} · ${T.file}` : T.file;
    const facts = h("dl", { class: "facts" }, h("dt", null, T.fType), h("dd", null, kind));
    if (!r.folder) facts.append(h("dt", null, T.fSize), h("dd", { class: "num" }, size(r.size)));
    facts.append(h("dt", null, T.fMod), h("dd", { class: "num" }, when(r.mtime)));
    const btn = (cls, ic, label, op, extra) => {
      const b = h("button", Object.assign({ class: "btn " + cls, type: "button" }, extra), icon(ic));
      if (label) b.append(label);
      b.addEventListener("click", () => act(op, S.rows[S.sel]));
      return b;
    };
    const open = btn("primary", I.open, T.open + " ", "open");
    open.append(h("kbd", null, "Enter"));
    pv.replaceChildren(
      h("div", { class: "thumb" }, badge(r)),
      h("div", null, h("div", { class: "pv-name" }, nameEl(r)), h("div", { class: "pv-path" }, pathEl(joinPath(r.dir, plainName(r)), 999))),
      h("div", { class: "actions" }, open, btn("", I.folder, T.openFolder, "reveal"), btn("icon", I.copy, null, "copy", { "aria-label": T.copyPath, title: T.copyPath })),
      facts,
    );
  }
  function renderRecent() {
    const el = $("recent");
    el.replaceChildren(...S.recent.map((r, i) => rowEl(r, i, "recent")));
    $("recent-none").hidden = S.recent.length > 0;
  }
  const SUGG_ICON = { query: I.bulb, all_kinds: I.filter, all_drives: I.folder, web: I.globe };
  function renderNoResults() {
    $("nores-h").textContent = T.noresH(S.q.trim());
    let sub = T.noresSub(S.files, S.kind ? S.kindLabel : "");
    if (S.state !== "ready") sub += " " + T.noresIx;
    $("nores-sub").textContent = sub;
    $("sugg").replaceChildren(...S.sugg.map((s, k) => {
      const el = h("div", { class: "sg", role: "option", id: "s" + k, "aria-selected": String(k === S.ssel) },
        h("span", { class: "ic" }, icon(SUGG_ICON[s.kind] || I.search)),
        h("span", { class: "tx" }, h("b", null, s.label), h("span", null, T.sugg[s.kind] || "")),
        k === S.ssel ? h("kbd", null, "Enter") : null);
      el.dataset.k = k;
      return el;
    }));
    if (S.sugg.length) q.setAttribute("aria-activedescendant", "s" + S.ssel); else q.removeAttribute("aria-activedescendant");
  }
  function applySuggestion(s) {
    if (!s) return;
    if (s.kind === "query") { q.value = s.q; onInput(); }
    else if (s.kind === "all_kinds") setKind(0);
    else if (s.kind === "web") send({ op: "web", q: S.q });
  }
  function renderBanner() {
    const busy = S.state !== "ready";
    $("banner").hidden = !busy;
    if (!busy) return;
    $("banner-title").textContent = S.state === "loading" ? T.loading : digits(S.stateText);
    $("banner-sub").textContent = T.ixSub;
    $("banner-bar").classList.add("busy");
  }
  function renderStatus() {
    const busy = S.state !== "ready";
    $("dot").classList.toggle("busy", busy);
    let text;
    if (S.notice) text = S.notice;
    else if (busy) text = S.state === "loading" ? T.loading : digits(S.stateText);
    else text = T.files(S.files);
    if (!S.notice && S.view !== "empty") text = (S.note ? S.note + " · " : "") + T.count(S.total, S.ms) + " · " + text;
    $("status-text").textContent = text;
  }
  // The result count for screen readers, once typing pauses.
  let liveTimer = 0;
  function announce() {
    clearTimeout(liveTimer);
    liveTimer = setTimeout(() => { $("live").textContent = T.nres(S.total); }, 500);
  }

  // ---------- the type chip ----------
  function renderKind() {
    const chip = $("kind"), label = $("kind-label");
    const on = S.kind > 0 && S.kinds[S.kind];
    chip.classList.toggle("on", !!on);
    chip.setAttribute("aria-pressed", String(!!on));
    label.textContent = on ? `${T.type}: ${S.kinds[S.kind]}` : T.type;
    $("kind-caret").replaceWith(Object.assign(icon(on ? I.x : I.down, "i caret"), { id: "kind-caret" }));
    const menu = $("kind-menu");
    menu.replaceChildren(...S.kinds.map((k, i) => {
      const it = h("div", { role: "menuitemradio", tabindex: "-1", "aria-checked": String(i === S.kind) }, k);
      it.dataset.k = i;
      return it;
    }));
  }
  function setKind(k) {
    S.kind = k; S.kindLabel = S.kinds[k] || "";
    closeMenu(); renderKind();
    q.focus();
    onInput();
  }
  function openMenu() {
    const menu = $("kind-menu");
    menu.hidden = false; $("kind").setAttribute("aria-expanded", "true");
    (menu.querySelector('[aria-checked="true"]') || menu.firstChild)?.focus();
  }
  function closeMenu() { $("kind-menu").hidden = true; $("kind").setAttribute("aria-expanded", "false"); }
  $("kind").addEventListener("click", (e) => {
    // The × of an active filter clears it; the rest opens the list.
    if (S.kind && e.target.closest("#kind-caret")) { setKind(0); return; }
    if ($("kind-menu").hidden) openMenu(); else closeMenu();
  });
  $("kind-menu").addEventListener("click", (e) => { const it = e.target.closest("[data-k]"); if (it) setKind(+it.dataset.k); });
  $("kind-menu").addEventListener("keydown", (e) => {
    const items = [...$("kind-menu").children], i = items.indexOf(document.activeElement);
    if (e.key === "ArrowDown") { items[(i + 1) % items.length].focus(); e.preventDefault(); }
    else if (e.key === "ArrowUp") { items[(i - 1 + items.length) % items.length].focus(); e.preventDefault(); }
    else if (e.key === "Enter" || e.key === " ") { if (i >= 0) setKind(i); e.preventDefault(); }
    else if (e.key === "Escape") { closeMenu(); $("kind").focus(); e.preventDefault(); e.stopPropagation(); }
  });
  document.addEventListener("mousedown", (e) => { if (!e.target.closest(".chipwrap")) closeMenu(); });

  // ---------- input ----------
  q.addEventListener("input", onInput);
  // Clicking a row keeps the caret in the search box.
  for (const el of [list, $("recent"), $("sugg")]) el.addEventListener("mousedown", (e) => { if (e.target.closest(".row,.sg")) e.preventDefault(); });
  list.addEventListener("click", (e) => { const r = e.target.closest(".row"); if (r) select(+r.dataset.i); });
  list.addEventListener("dblclick", (e) => { const r = e.target.closest(".row"); if (r) act("open", S.rows[+r.dataset.i]); });
  list.addEventListener("scroll", () => { if (list.scrollTop + list.clientHeight > list.scrollHeight - FIRST_PAGE_GUARD) askMore(); });
  $("recent").addEventListener("click", (e) => { const r = e.target.closest(".row"); if (r) act("open", S.recent[+r.dataset.i], "recent"); });
  $("sugg").addEventListener("click", (e) => { const s = e.target.closest(".sg"); if (s) applySuggestion(S.sugg[+s.dataset.k]); });
  $("tips").addEventListener("click", (e) => { const c = e.target.closest("code"); if (c) { q.value = c.textContent; q.focus(); onInput(); } });

  document.addEventListener("keydown", (e) => {
    if (!$("kind-menu").hidden && $("kind-menu").contains(document.activeElement)) return;
    const ctrl = e.ctrlKey || e.metaKey;
    if (ctrl && (e.key === "k" || e.key === "K" || e.key === "f" || e.key === "F")) { q.focus(); q.select(); e.preventDefault(); return; }
    if (e.key === "Escape") {
      if (!$("kind-menu").hidden) closeMenu();
      else if (q.value) { q.value = ""; onInput(); q.focus(); }
      else send({ op: "hide" });
      e.preventDefault();
      return;
    }
    if (S.view === "nores" && S.sugg.length) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        S.ssel = (S.ssel + (e.key === "ArrowDown" ? 1 : S.sugg.length - 1)) % S.sugg.length;
        renderNoResults(); e.preventDefault();
      } else if (e.key === "Enter" && document.activeElement === q) { applySuggestion(S.sugg[S.ssel]); e.preventDefault(); }
      return;
    }
    if (S.view !== "results") return;
    const page = Math.max(1, Math.floor(list.clientHeight / 56) - 1);
    const moves = { ArrowDown: 1, ArrowUp: -1, PageDown: page, PageUp: -page };
    if (e.key in moves && (document.activeElement === q || document.activeElement === document.body)) { select(S.sel + moves[e.key]); e.preventDefault(); return; }
    if (ctrl && (e.key === "Home" || e.key === "End")) { select(e.key === "Home" ? 0 : S.rows.length - 1); e.preventDefault(); return; }
    if (e.key === "Enter" && (document.activeElement === q || document.activeElement === document.body)) {
      act(ctrl ? "reveal" : "open", S.rows[S.sel]); e.preventDefault(); return;
    }
    if (ctrl && e.shiftKey && (e.key === "c" || e.key === "C")) { act("copy", S.rows[S.sel]); e.preventDefault(); }
  });

  // ---------- start ----------
  for (const el of document.querySelectorAll("[data-i18n]")) el.textContent = T[el.dataset.i18n];
  for (const el of document.querySelectorAll("[data-i18n-placeholder]")) el.placeholder = T[el.dataset.i18nPlaceholder];
  for (const el of document.querySelectorAll("[data-i18n-label]")) el.setAttribute("aria-label", T[el.dataset.i18nLabel]);
  $("tips").replaceChildren(...T.tips.map(([c, s]) => h("div", { class: "tip" }, h("code", { dir: "auto" }, c), h("span", null, s))));
  setView("empty");
  renderRecent();
  renderStatus();
  q.focus();
  send({ op: "hello" });
})();
