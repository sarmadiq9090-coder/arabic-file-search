// Renders the new main window's page (src/webui/assets) in headless Chromium
// with a fake program behind window.ipc and synthetic files, checks it, and
// saves screenshots. Development tool only: not part of the exe or of CI.
//
//   NODE_PATH=<folder with playwright> node src/webui/preview.cjs <outDir> [chromium path]
//
// The page is served exactly as the exe serves it (same URL, same CSP
// header), so a CSP violation, a request to anywhere else, a script error,
// text that reads "undefined"/"NaN", a menu or dialog off screen, or a
// horizontal overflow fails the run. It also checks that every path is laid
// out left to right with its folder names in order (in Arabic too), that the
// page follows the language's direction, that a file name shows its
// extension left to right at its logical end (".xlsx" on the left of an
// Arabic name, on the right of an English one) with the matches still
// marked, that phone numbers and dates read as typed (0770-123-4567 after
// Arabic words, not 4567-123-0770), and that every message the page
// sends has the fields src/webui/bridge.rs accepts and nothing more (the
// program refuses unknown fields), and that every request about a row names
// the version of the list it is on (the fake numbers its lists as
// bridge.rs `Lists` does, and answers a request about an older one as the
// program does: "the list changed").
"use strict";
const { chromium } = require("playwright");
const fs = require("fs");
const path = require("path");

const ASSETS = path.join(__dirname, "assets");
const ORIGIN = "http://instafind.localhost";
const out = path.resolve(process.argv[2] || "webui-shots");
const exe = process.argv[3];
fs.mkdirSync(out, { recursive: true });

// The CSP, read from assets.rs so the preview can't drift from the exe.
const rs = fs.readFileSync(path.join(__dirname, "assets.rs"), "utf8");
const CSP = rs.match(/pub const CSP: &str = "([^;]*;[\s\S]*?)";/)[1].replace(/\\\n/g, "");

// A 4x4 opaque PNG, the program's thumbnails' format.
const TEST_PNG = (() => {
  const zlib = require("zlib");
  const chunk = (type, data) => {
    const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
    const td = Buffer.concat([Buffer.from(type), data]);
    const crc = Buffer.alloc(4); crc.writeUInt32BE(zlib.crc32(td) >>> 0);
    return Buffer.concat([len, td, crc]);
  };
  const ihdr = Buffer.from([0, 0, 0, 4, 0, 0, 0, 4, 8, 6, 0, 0, 0]);
  const raw = Buffer.concat(Array.from({ length: 4 }, () => Buffer.from([0, ...[].concat(...Array(4).fill([47, 100, 214, 255]))])));
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", zlib.deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
})();
const MIME = { ".js": "text/javascript", ".css": "text/css", ".woff2": "font/woff2" };
function page(lang) {
  return fs.readFileSync(path.join(ASSETS, "index.html"), "utf8")
    .replace("{{LANG}}", lang).replace("{{DIR}}", lang === "ar" ? "rtl" : "ltr")
    .replace("{{CSP}}", CSP.replace("; frame-ancestors 'none'", "")).replace("{{TOKEN}}", "00112233445566778899aabbccddeeff");
}

// The fake program: runs inside the page, answers like src/webui/bridge.rs.
function fakeProgram(cfg) {
  const now = Math.floor(Date.now() / 1000);
  const U = "C:\\Users\\Sara";
  const ar = cfg.lang === "ar";
  window.__sent = []; window.__fakeErrors = []; window.__stale = [];
  const files = (ar ? [
    [`${U}\\Documents\\العمل\\المالية`, "تقرير المبيعات ٢٠٢٦.xlsx", 48 * 1024, 7 * 60],
    [`${U}\\Documents\\التقارير\\2026`, "التقرير السنوي النهائي.pdf", 2.4 * 1048576, 26 * 3600],
    [`${U}\\Desktop`, "تقرير الاجتماع الأسبوعي.docx", 31 * 1024, 5 * 3600],
    [`${U}\\Documents\\العمل`, "خطة الميزانية والتقرير.docx", 64 * 1024, 3 * 86400],
    ["D:\\Projects\\Field-2026\\Basra", "صور التقرير الميداني.zip", 184 * 1048576, 4 * 86400],
    ["D:\\Projects\\Maintenance\\عروض", "تقرير_الصيانة_Q3.pptx", 6.1 * 1048576, 11 * 86400],
    [`${U}\\Pictures\\تصاميم`, "غلاف التقرير.png", 1.2 * 1048576, 13 * 86400],
    [`${U}\\Documents`, "ملاحظات التقرير.txt", 2 * 1024, 18 * 86400],
    [`${U}\\OneDrive\\الموارد البشرية\\الحضور والانصراف\\2026`, "تقرير الحضور - آذار.xlsx", 92 * 1024, 200 * 86400],
    [`${U}\\Music\\تسجيلات`, "تسجيل التقرير الصوتي.m4a", 8.7 * 1048576, 240 * 86400],
    ["D:\\Archive\\2024", "تقارير قديمة", 0, 400 * 86400, true],
    // A phone number and a date after Arabic words: each must read as typed.
    [`${U}\\Documents\\الفواتير 2026-11`, "فاتورة رقم 1234 بتاريخ 2026-11-12 من شركة Microsoft هاتف 0770-123-4567.txt", 3 * 1024, 30 * 86400],
  ] : [
    [`${U}\\Documents\\Work\\Finance`, "Sales report 2026.xlsx", 48 * 1024, 7 * 60],
    [`${U}\\Documents\\Reports\\2026`, "Annual report final.pdf", 2.4 * 1048576, 26 * 3600],
    [`${U}\\Desktop`, "Weekly meeting report.docx", 31 * 1024, 5 * 3600],
    [`${U}\\Documents\\Work`, "Budget plan and report.docx", 64 * 1024, 3 * 86400],
    ["D:\\Projects\\Field-2026\\Basra", "Field report photos.zip", 184 * 1048576, 4 * 86400],
    ["D:\\Projects\\Maintenance\\Decks", "Maintenance_report_Q3.pptx", 6.1 * 1048576, 11 * 86400],
    [`${U}\\Pictures\\Designs`, "Report cover.png", 1.2 * 1048576, 13 * 86400],
    [`${U}\\Documents`, "Report notes.txt", 2 * 1024, 18 * 86400],
    [`${U}\\OneDrive\\HR\\Attendance\\2026`, "Attendance report - March.xlsx", 92 * 1024, 200 * 86400],
    [`${U}\\Music\\Recordings`, "Report voice memo.m4a", 8.7 * 1048576, 240 * 86400],
    ["D:\\Archive\\2024", "Old reports", 0, 400 * 86400, true],
  ]).map(([dir, name, size, age, folder]) => ({ dir, name, size: Math.round(size), mtime: now - age, folder: !!folder, ext: folder ? "" : name.split(".").pop().toLowerCase() }));
  // Synthetic document texts for the search inside documents.
  const docs = ar ? [
    [`${U}\\Documents\\العقود`, "عقد إيجار الشقة 2026.docx", 88 * 1024, 2 * 86400, "…يلتزم المستأجر بدفع بدل عقد الإيجار في أول كل شهر، ويُجدَّد عقد الإيجار تلقائياً…", 6],
    [`${U}\\Downloads`, "نموذج عقد إيجار محل تجاري.pdf", 410 * 1024, 9 * 86400, "نموذج عقد الإيجار التجاري: الطرف الأول (المؤجر) والطرف الثاني (المستأجر)…", 3],
    [`${U}\\Pictures\\ممسوحة`, "صورة العقد الموقّع.jpg", 2.1 * 1048576, 30 * 86400, "…وقّع الطرفان على عقد الإيجار بتاريخ 2026/03/01 أمام الشاهدين…", 1],
  ] : [
    [`${U}\\Documents\\Contracts`, "Apartment lease 2026.docx", 88 * 1024, 2 * 86400, "…the tenant pays the rent under this lease agreement on the first day of each month; the lease agreement renews…", 6],
    [`${U}\\Downloads`, "Shop lease agreement template.pdf", 410 * 1024, 9 * 86400, "Commercial lease agreement template: the first party (landlord) and the second party (tenant)…", 3],
    [`${U}\\Pictures\\Scans`, "Signed contract scan.jpg", 2.1 * 1048576, 30 * 86400, "…both parties signed the lease agreement on 2026-03-01 before two witnesses…", 1],
  ];
  const KINDS = ar ? ["كل الأنواع", "مستندات", "صور", "صوت", "فيديو", "مضغوطات", "برامج", "مجلدات"] : ["All types", "Documents", "Pictures", "Audio", "Video", "Archives", "Programs", "Folders"];
  const SIZES = ar ? ["أي حجم", "أصغر من 100 كيلوبايت", "من 100 كيلوبايت إلى 10 ميغابايت", "أكبر من 10 ميغابايت", "أكبر من 1 غيغابايت"] : ["Any size", "Under 100 KB", "100 KB to 10 MB", "Over 10 MB", "Over 1 GB"];
  const DATES = ar ? ["أي وقت", "اليوم", "آخر 7 أيام", "آخر 30 يوماً", "هذه السنة", "أقدم من سنة"] : ["Any time", "Today", "Last 7 days", "Last 30 days", "This year", "Older than a year"];
  const DOCS = ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt"];
  const SIZE_OK = [() => true, (f) => f.size < 100 * 1024, (f) => f.size >= 100 * 1024 && f.size <= 10 * 1048576, (f) => f.size > 10 * 1048576, (f) => f.size > 1 << 30];
  const DATE_OK = [() => true, (f) => now - f.mtime < 86400, (f) => now - f.mtime < 7 * 86400, (f) => now - f.mtime < 30 * 86400, (f) => now - f.mtime < 200 * 86400, (f) => now - f.mtime > 365 * 86400];
  // The fields each request may carry (bridge.rs `Request`, deny_unknown_fields).
  const FIELDS = {
    hello: [], search: ["seq", "q", "kind", "size", "date", "content", "sort", "desc"], more: ["seq", "ver", "from"], sort: ["seq", "sort", "desc"], stop: ["seq"],
    matches: ["seq", "ver", "id"], open: ["list", "seq", "ver", "id"], reveal: ["list", "seq", "ver", "id"], copy: ["list", "seq", "ver", "id"], copy_file: ["list", "seq", "ver", "id"], copy_all: ["seq", "ver"],
    rename: ["list", "seq", "ver", "id", "name"], recycle: ["list", "seq", "ver", "id"], scope: ["list", "seq", "ver", "id"], pick_folder: [], all_drives: [], saved: ["id", "name"],
    save_search: ["q", "kind", "content"], clear_recent: [], clear_saved: [], set: ["key", "on"], lang: ["lang"], restart: [], reindex: [], web: ["q"], hide: [], options_menu: [], report_problem: [], drag: ["list", "seq", "ver", "id"], app: ["seq", "id"], thumbs: ["seq", "ver", "ids"], export: [], replace_text: [], folder_sizes: ["list", "seq", "ver", "id"],
    switch_to: ["list", "seq", "ver", "id"], switch_cancel: [], open_with: ["list", "seq", "ver", "id"], properties: ["list", "seq", "ver", "id"],
  };
  const REQUIRED = { search: ["seq", "q"], sort: ["seq", "sort"], set: ["key", "on"], rename: ["list", "seq", "ver", "id", "name"] };
  // Requests about rows, and the list version they must name (a new one for every list sent and every new order).
  const ROW_OPS = ["thumbs", "more", "matches", "open", "reveal", "copy", "copy_file", "copy_all", "rename", "recycle", "scope", "drag", "folder_sizes", "switch_to", "open_with", "properties"];
  let lastVer = 0, recentVer = 0;
  const nextVer = () => ++lastVer;
  const send = (m) => setTimeout(() => window.__instafind.recv(m), 0);
  const mark = (text, term) => {
    const out = [], lower = text.toLowerCase(), t = term.toLowerCase();
    let at = 0;
    for (let i = lower.indexOf(t); t && i >= 0; i = lower.indexOf(t, i + t.length)) { if (i > at) out.push([text.slice(at, i), false]); out.push([text.slice(i, i + t.length), true]); at = i + t.length; }
    if (at < text.length || !out.length) out.push([text.slice(at), false]);
    return out;
  };
  const row = (f, id, term) => {
    const r = { id, name: mark(f.name, term || ""), dir: f.dir, size: f.size, mtime: f.mtime, ext: f.ext, folder: f.folder };
    if (f.snippet) { r.snippet = mark(f.snippet, term); r.matches = f.count; }
    return r;
  };
  const status = cfg.indexing
    ? { type: "status", state: "names", files: 36000, text: ar ? "جارٍ الفهرسة… 36,000 ملف حتى الآن" : "Indexing… 36,000 files so far" }
    : { type: "status", state: "ready", files: 1204331, text: null };
  const settings = { type: "settings", lang: cfg.lang, next_lang: cfg.lang, digits: false, autostart: true, content_index: true, read_cloud_files: false, auto_index: true, hide_system: true, hotkey_quick: true, floating: true, tray_icon: true, version: "0.9.1" };
  const history = {
    type: "history",
    recent: ar ? ["تقرير المبيعات", "عقد الإيجار", "فاتورة الكهرباء .pdf", "صور العيد"] : ["sales report", "lease agreement", "electricity bill .pdf", "holiday photos"],
    saved: [{ id: 0, name: ar ? "عقود العمل" : "Work contracts", content: true, scoped: true }, { id: 1, name: ar ? "تقارير هذا الشهر" : "Reports this month", content: false, scoped: false }],
    clearable: true,
  };
  if (cfg.noSearches) history.recent = [];
  let shown = { seq: 0, ver: 0, list: [], term: "" };
  const SORT = { name: (a, b) => a.name.localeCompare(b.name), path: (a, b) => (a.dir + a.name).localeCompare(b.dir + b.name), size: (a, b) => a.size - b.size, date: (a, b) => a.mtime - b.mtime };
  const order = (list, sort, desc) => { if (sort === "best") return list; const s = [...list].sort(SORT[sort]); return desc ? s.reverse() : s; };
  const results = (m, extra) => ({ type: "results", seq: m.seq, ver: shown.ver, total: shown.list.length, ms: 18, rows: shown.list.slice(0, 60).map((f, i) => row(f, i, shown.term)), note: null, suggestions: [], files: status.files, kind_label: m.kind ? KINDS[m.kind] : "", scope: cfg.scope || "", content: false, running: false, warnings: [], ...extra });
  Object.defineProperty(window, "ipc", { value: { postMessage(s) {
    const m = JSON.parse(s);
    const fail = (why) => window.__fakeErrors.push(`${why}: ${s}`);
    if (m.t !== "00112233445566778899aabbccddeeff") fail("no token");
    const allowed = FIELDS[m.op];
    if (!allowed) return fail("unknown op");
    for (const k of Object.keys(m)) if (k !== "t" && k !== "op" && !allowed.includes(k)) fail(`unknown field ${k}`);
    for (const k of REQUIRED[m.op] || allowed) if (!(k in m)) fail(`missing field ${k}`);
    window.__sent.push(m);
    if (ROW_OPS.includes(m.op)) {
      const current = m.list === "recent" ? m.ver === recentVer && recentVer > 0 : m.seq === shown.seq && m.ver === shown.ver;
      if (!current) {
        // As the program: "more" and "matches" of an older list are dropped, the rest refused with a word.
        if (m.op !== "more" && m.op !== "matches" && m.op !== "thumbs") { window.__stale.push(m.op); send({ type: "notice", text: ar ? "تغيّرت القائمة للتو، فلم يُنفَّذ شيء. حاول مرة أخرى." : "The list just changed, so nothing was done. Try again." }); }
        return;
      }
    }
    if (m.op === "hello") {
      send({ type: "hello", kinds: KINDS, sizes: SIZES, dates: DATES, kind: 0, sort: "best", desc: false, digits: false, version: "0.9.1", switched: !!cfg.switched });
      send({ type: "scope", path: cfg.scope || "", search: false });
      send(settings); send(status); send(history);
      recentVer = nextVer();
      send({ type: "recent", ver: recentVer, rows: files.slice(0, 6).map((f, i) => row(f, i, "")) });
    } else if (m.op === "search" && m.content) {
      const words = m.q.trim().split(/\s+/);
      const term = ar ? (m.q.includes("الإيجار") ? "الإيجار" : words[0]) : (m.q.toLowerCase().includes("lease") ? "lease agreement" : words[0]);
      const found = docs.map(([dir, name, size, age, snippet, count]) => ({ dir, name, size, mtime: now - age, folder: false, ext: name.split(".").pop(), snippet, count }))
        .filter((f) => f.snippet.toLowerCase().includes(term.toLowerCase()));
      shown = { seq: m.seq, ver: nextVer(), list: [], term, content: true };
      send(results(m, { content: true, running: true, total: 0, rows: [] }));
      setTimeout(() => {
        shown.list = order(found, m.sort, m.desc);
        send({ type: "progress", seq: m.seq, total: found.length, done: false, text: ar ? "جارٍ البحث داخل المستندات… تم فحص 1,840 من 4,210 — وجدت 3" : "Searching inside documents… checked 1,840 of 4,210 — found 3" });
        setTimeout(() => {
          // Put in order at the end: a new version, as in the program.
          shown.ver = nextVer();
          send(results(m, { content: true, running: cfg.keepRunning, suggestions: found.length ? [] : [{ kind: "web", label: ar ? "ابحث في الويب" : "Search the web", q: "" }] }));
          if (!cfg.keepRunning) send({ type: "progress", seq: m.seq, total: found.length, done: true, text: ar ? "انتهى البحث داخل 4,210 مستند: وجدت 3 ملف. (من الفهرس: 4,102)" : "Searched 4,210 documents: found 3 files. (4,102 from the index)" });
        }, 40);
      }, 40);
    } else if (m.op === "search") {
      // The apps the text names (the program's catalog, faked).
      send({ type: "apps", seq: m.seq, apps: /chrome|كروم/i.test(m.q) ? ["Google Chrome"] : [] });
      const words = m.q.trim().split(/\s+/);
      const term = words[0] === "تقرير" ? "تقرير" : words[0];
      let hits = files.filter((f) => f.name.toLowerCase().includes(term.toLowerCase()) && SIZE_OK[m.size || 0](f) && DATE_OK[m.date || 0](f));
      if (m.kind === 1) hits = hits.filter((f) => DOCS.includes(f.ext));
      if (cfg.scope) hits = hits.filter((f) => f.dir.startsWith(cfg.scope.replace(/\\$/, "")));
      if (cfg.indexing) hits = hits.slice(0, 4);
      shown = { seq: m.seq, ver: nextVer(), list: order(hits, m.sort || "best", m.desc), term };
      const n = cfg.indexing || cfg.scope || m.size || m.date ? hits.length : hits.length ? Math.max(hits.length, 312) : 0;
      const sugg = hits.length ? [] : [
        { kind: "query", label: ar ? "هل تقصد «عقد الإيجار 2023»؟" : "Did you mean “lease 2023”?", q: ar ? "عقد الإيجار 2023" : "lease 2023" },
        ...(m.kind ? [{ kind: "all_kinds", label: ar ? "ابحث في كل أنواع الملفات" : "Search all file types", q: "" }] : []),
        { kind: "contents", label: ar ? "ابحث داخل محتوى الملفات" : "Search inside file contents", q: "" },
        { kind: "web", label: ar ? "ابحث في الويب" : "Search the web", q: "" },
      ];
      send(results(m, { total: n, suggestions: sugg }));
    } else if (m.op === "thumbs") {
      // One-time keys by row (even rows have a picture, odd ones none: 404).
      if (!Array.isArray(m.ids) || !m.ids.length || m.ids.length > 64) fail("bad thumbs ids");
      const hex = () => Array.from({ length: 31 }, () => "0123456789abcdef"[Math.floor(Math.random() * 16)]).join("");
      send({ type: "thumbs", seq: m.seq, ver: m.ver, keys: m.ids.map((id) => [id, (id % 2 ? "b" : "a") + hex()]) });
    } else if (m.op === "more") {
      send({ type: "rows", seq: m.seq, ver: m.ver, from: m.from, rows: shown.list.slice(m.from, m.from + 200).map((f, i) => row(f, m.from + i, shown.term)) });
    } else if (m.op === "sort") {
      shown.list = order(shown.list, m.sort, m.desc);
      shown.ver = nextVer();
      send(results(m, { content: !!shown.content }));
    } else if (m.op === "matches") {
      const f = shown.list[m.id];
      if (f && f.snippet) send({ type: "matches", seq: m.seq, ver: m.ver, id: m.id, count: f.count, lines: Array.from({ length: Math.min(f.count, 6) }, (_, k) => [4 + k * 7, mark(k ? f.snippet.replace("…", "") : f.snippet, shown.term)]) });
    } else if (m.op === "rename") {
      send({ type: "renamed", list: m.list, seq: m.seq, ver: m.ver, id: m.id, name: m.name });
      send({ type: "notice", text: (ar ? "تم تغيير الاسم إلى «" : "Renamed to “") + m.name + (ar ? "»." : "”.") });
    } else if (m.op === "recycle") {
      send({ type: "removed", list: m.list, seq: m.seq, ver: m.ver, id: m.id });
    } else if (["copy", "copy_file", "copy_all", "open", "reveal"].includes(m.op)) {
      send({ type: "notice", text: ar ? "تم نسخ المسار." : "Path copied." });
    } else if (m.op === "pick_folder" || m.op === "all_drives" || m.op === "scope") {
      cfg.scope = m.op === "all_drives" ? "" : m.op === "scope" ? shown.list[m.id].dir + "\\" : "D:\\Projects\\";
      send({ type: "scope", path: cfg.scope, search: true });
    } else if (m.op === "saved") {
      cfg.scope = m.id === 0 ? `${U}\\Documents\\${ar ? "العقود" : "Contracts"}\\` : "";
      send({ type: "scope", path: cfg.scope, search: false });
      send({ type: "apply", q: ar ? "عقد العمل" : "employment contract", kind: 1, content: m.id === 0 });
    } else if (m.op === "set") {
      settings[m.key] = m.on; send({ ...settings });
    } else if (m.op === "lang") {
      settings.next_lang = m.lang; send({ ...settings });
    } else if (m.op === "switch_to" || m.op === "switch_cancel") {
      // The dialog was moved (or forgotten): the mode is over.
      send({ type: "quick_switch", on: false, text: "" });
      if (m.op === "switch_cancel") send({ type: "notice", text: ar ? "أُلغي نقل نافذة الحفظ/الفتح." : "Save/Open dialog switch cancelled.", warn: false });
    } else if (m.op === "save_search" || m.op === "clear_recent" || m.op === "clear_saved") {
      // As History::clear_activity: the opened files go too.
      if (m.op === "clear_recent") { history.recent = []; history.clearable = false; recentVer = nextVer(); send({ type: "recent", ver: recentVer, rows: [] }); }
      send({ ...history });
    }
  } } });
  // The name search run again in the background (the index changed), in another order.
  window.__rerun = () => { shown.list = [...shown.list].reverse(); shown.ver = nextVer(); send(results({ seq: shown.seq, kind: 0 }, {})); };
}

// The program's word that a Save/Open dialog waits (Ctrl+Alt+G).
const QS_ON = {
  ar: "window.__instafind.recv({ type: 'quick_switch', on: true, text: 'Enter: نقل النافذة إلى مجلد النتيجة · Esc: إلغاء' })",
  en: "window.__instafind.recv({ type: 'quick_switch', on: true, text: 'Enter: move the dialog to the result\\'s folder · Esc: cancel' })",
};
// name, lang, theme, steps (run in the page with Playwright), fake program options
const SHOTS = [
  ["main-ar-light", "ar", "light", [["type", "تقرير"], ["key", "ArrowDown"]]],
  ["main-en-dark", "en", "dark", [["type", "report"], ["key", "ArrowDown"]]],
  ["empty-ar-light", "ar", "light", []],
  ["empty-en-dark", "en", "dark", []],
  ["nores-ar-dark", "ar", "dark", [["chip", "kind", 1], ["type", "عقد الايجار 2024"]]],
  ["nores-en-light", "en", "light", [["type", "lease 2024"]]],
  ["indexing-ar-light", "ar", "light", [["type", "تقرير"]], { indexing: true }],
  ["content-ar-light", "ar", "light", [["click", "#chip-content"], ["type", "عقد الإيجار"], ["key", "Enter"], ["wait", 250], ["key", "ArrowDown"], ["key", "ArrowUp"], ["wait", 250]], { expect: { content: true, rows: 3 } }],
  ["content-en-dark", "en", "dark", [["click", "#chip-content"], ["type", "lease agreement"], ["key", "Enter"], ["wait", 250], ["wait", 250]], { expect: { content: true, rows: 3 } }],
  ["content-running-ar-dark", "ar", "dark", [["click", "#chip-content"], ["type", "عقد الإيجار"], ["key", "Enter"], ["wait", 250]], { keepRunning: true, expect: { stop: true } }],
  ["matches-ar-light", "ar", "light", [["click", "#chip-content"], ["type", "عقد الإيجار"], ["key", "Enter"], ["wait", 300], ["key", "F4"], ["wait", 150]], { expect: { dialog: true, dlgName: true } }],
  // Enter in the box moves to the list (as the classic window); Enter there opens.
  ["enter-en-light", "en", "light", [["type", "report"], ["key", "Enter"]], { expect: { focus: "list", opens: 0 } }],
  ["enter-twice-en-light", "en", "light", [["type", "report"], ["key", "Enter"], ["key", "Enter"]], { expect: { opens: 1 } }],
  // The first indexing, nothing found yet: "still indexing", not "nothing in 0 files".
  ["indexing-nores-ar-light", "ar", "light", [["type", "zzqq"]], { indexing: true, expect: { view: "view-nores", stillIndexing: true, tagline: false } }],
  // A classic user moved by the update: told once (no way back since 1.0).
  ["switched-ar-light", "ar", "light", [], { switched: true, expect: { switched: true } }],
  ["switched-dismiss-en-light", "en", "light", [["click", "#switched-x"], ["wait", 50]], { switched: true, expect: { switched: false } }],
  // A problem stays, in the warning colour, until the next key.
  ["warn-en-dark", "en", "dark", [["eval", "window.__instafind.recv({ type: 'notice', text: 'Could not open the file: C:\\\\x.txt', warn: true })"], ["wait", 3800]], { expect: { warn: true } }],
  ["warn-gone-en-dark", "en", "dark", [["eval", "window.__instafind.recv({ type: 'notice', text: 'Could not open the file', warn: true })"], ["key", "Shift"]], { expect: { warn: false } }],
  // Density: compact rows by themselves in a low window (one line, ≤ 36 px), the preview pane can be hidden,
  // and the indexing banner shrinks to one line after its first seconds.
  ["compact-ar-light", "ar", "light", [["type", "تقرير"], ["key", "ArrowDown"]], { height: 700, expect: { compact: true, rowH: true } }],
  ["roomy-en-light", "en", "light", [["type", "report"]], { height: 900, expect: { compact: false } }],
  ["compact-chosen-en-dark", "en", "dark", [["type", "report"], ["click", "#view-compact"]], { height: 900, expect: { compact: true, rowH: true } }],
  ["no-preview-ar-light", "ar", "light", [["type", "تقرير"], ["click", "#view-preview"]], { expect: { preview: false } }],
  ["banner-slim-ar-light", "ar", "light", [["type", "تقرير"], ["wait", 6500]], { indexing: true, expect: { bannerSlim: true } }],
  // One-click actions: Export and Update index in the toolbar, labelled History/Settings, the
  // right-click menu's folder sizes and replace text, and the actions under the list in a narrow window.
  ["tools-en-light", "en", "light", [["type", "report"], ["click", "#tool-export"]], { expect: { lastSent: "export", labels: true } }],
  ["ctx-sizes-ar-light", "ar", "light", [["type", "تقرير"], ["rclick", "#r1"], ["click", "#ctx [data-op=folder_sizes]"], ["wait", 50]], { expect: { lastSent: "folder_sizes" } }],
  ["narrow-ar-light", "ar", "light", [["type", "تقرير"], ["key", "ArrowDown"]], { width: 820, expect: { actbar: 4, labels: false } }],
  ["actbar-no-preview-en-light", "en", "light", [["type", "report"], ["click", "#view-preview"], ["click", "#actbar .btn:nth-child(3)"], ["wait", 50]], { expect: { actbar: 4, lastSent: "copy" } }],
  // The classic window's terms, and the date with its time in the preview.
  ["terms-ar-light", "ar", "light", [["type", "تقرير"]], { expect: { termsAr: true, dateTime: true } }],
  // Zoom with Ctrl and + / 0; off switches readable; paths 13.5 px.
  ["zoom-en-light", "en", "light", [["type", "report"], ["key", "Control+="], ["key", "Control+="]], { expect: { zoom: "1.25", pathPx: true } }],
  ["zoom-reset-en-light", "en", "light", [["key", "Control+="], ["key", "Control+0"]], { expect: { zoom: "1" } }],
  // A document found by its text: its matching lines start above the fold; wide screens.
  ["content-lines-en-light", "en", "light", [["click", "#chip-content"], ["type", "lease agreement"], ["key", "Enter"], ["wait", 300]], { expect: { linesTop: true } }],
  ["wide-en-light", "en", "light", [["type", "report"]], { width: 1920, height: 1080, expect: { rows: 11 } }],
  // Column widths: the edge before the Date header drags (kept in the page's storage).
  ["col-width-en-light", "en", "light", [["type", "report"], ["drag", ".cwrap:last-child .grip"]], { expect: { dateW: "98px" } }],
  ["col-width-ar-light", "ar", "light", [["type", "تقرير"], ["drag", ".cwrap:last-child .grip"]], { expect: { dateW: "158px" } }],
  // The thumbnail view (Ctrl+T): tiles, pictures by one-time key (never a path), keys in reading order.
  ["grid-ar-light", "ar", "light", [["type", "تقرير"], ["key", "Control+t"], ["wait", 500]], { expect: { grid: true, pics: true } }],
  ["grid-en-dark", "en", "dark", [["type", "report"], ["click", "#view-grid"], ["wait", 500]], { expect: { grid: true, pics: true } }],
  ["grid-keys-ar-light", "ar", "light", [["type", "تقرير"], ["key", "Control+t"], ["wait", 200], ["click", "#r0"], ["key", "ArrowLeft"]], { expect: { active: "r1" } }],
  ["grid-down-en-light", "en", "light", [["type", "report"], ["key", "Control+t"], ["wait", 200], ["click", "#r0"], ["key", "ArrowDown"]], { expect: { downByCols: true } }],
  ["grid-off-en-light", "en", "light", [["type", "report"], ["key", "Control+t"], ["key", "Control+t"]], { expect: { grid: false } }],
  // A Latin search in the Arabic interface stays at the right.
  ["latin-query-ar-light", "ar", "light", [["type", "report"]], { expect: { qAlign: "right" } }],
  ["filters-ar-light", "ar", "light", [["chip", "size", 2], ["chip", "date", 4], ["chip", "scope", 1], ["type", "تقرير"], ["wait", 100], ["click", '.colbtn[data-sort="size"]'], ["click", '.colbtn[data-sort="size"]'], ["key", "ArrowDown"]], { expect: { chipsOn: 3 } }],
  ["filters-en-light", "en", "light", [["chip", "kind", 1], ["chip", "date", 3], ["type", "report"], ["chip", "sort", 4], ["key", "ArrowDown"]], { expect: { chipsOn: 2 } }],
  ["sort-menu-en-dark", "en", "dark", [["type", "report"], ["click", "#chip-sort"]], { expect: { menu: true } }],
  ["ctx-ar-light", "ar", "light", [["type", "تقرير"], ["rclick", "#r1"]], { expect: { menu: true } }],
  ["ctx-en-dark", "en", "dark", [["type", "report"], ["key", "ArrowDown"], ["key", "Shift+F10"]], { expect: { menu: true } }],
  ["rename-ar-light", "ar", "light", [["type", "تقرير"], ["key", "F2"], ["wait", 50]], { expect: { dialog: true } }],
  // Delete in the search box edits the text; in the list it asks first, then recycles.
  ["rename-done-en-light", "en", "light", [["type", "report"], ["key", "F2"], ["fill", "#rename", "Sales report 2026 (final).xlsx"], ["key", "Enter"], ["wait", 50], ["key", "ArrowDown"], ["key", "Delete"], ["wait", 50], ["click", "#r1"], ["key", "Delete"], ["wait", 50], ["click", "#recycle-ok"], ["wait", 50]], { expect: { renamed: "Sales report 2026 (final).xlsx", gone: 1, recycles: 1, dialog: false } }],
  ["delete-in-box-en-dark", "en", "dark", [["type", "report"], ["key", "ArrowDown"], ["key", "Delete"], ["wait", 50]], { expect: { dialog: false, recycles: 0, q: "report" } }],
  ["recycle-ar-light", "ar", "light", [["type", "تقرير"], ["click", "#r1"], ["key", "Delete"], ["wait", 50]], { expect: { dialog: true, recycles: 0, focus: "recycle-ok" } }],
  ["recycle-cancel-ar-dark", "ar", "dark", [["type", "تقرير"], ["rclick", "#r2"], ["click", "#ctx [data-op=recycle]"], ["wait", 50], ["key", "Escape"], ["wait", 50]], { expect: { dialog: false, recycles: 0, gone: 0, focus: "list" } }],
  // Typing with the focus on the list goes on in the box.
  ["list-typing-en-light", "en", "light", [["type", "report"], ["click", "#r1"], ["key", "s"], ["wait", 200]], { expect: { q: "reports", focus: "q" } }],
  // The rename dialog names the list it was opened on: the list listed again
  // meanwhile, its OK is refused with a word, never applied to the file now at that place.
  ["rename-stale-en-light", "en", "light", [["type", "report"], ["key", "F2"], ["eval", "window.__rerun()"], ["wait", 50], ["fill", "#rename", "Other name.xlsx"], ["key", "Enter"], ["wait", 50]], { expect: { stale: 1, dialog: false } }],
  ["history-ar-light", "ar", "light", [["click", "#hist-btn"]], { expect: { menu: true } }],
  ["history-en-dark", "en", "dark", [["key", "ArrowDown"]], { expect: { menu: true } }],
  ["saved-ar-light", "ar", "light", [["click", "#hist-btn"], ["click", "#hist [role=menuitem]"], ["wait", 300]], { expect: { content: true } }],
  ["settings-ar-light", "ar", "light", [["click", "#set-btn"], ["click", "#settings .seg[lang=en]"], ["click", "#set-floating"]], { expect: { settings: true } }],
  ["settings-en-dark", "en", "dark", [["key", "Control+,"]], { expect: { settings: true } }],
  // Dragging a row out asks the program to drag that file (once), naming its list version.
  ["drag-en-light", "en", "light", [["type", "report"], ["drag", "#r1"]], { expect: { drags: 1, dialog: false } }],
  ["drag-recent-ar-light", "ar", "light", [["drag", "#rr0"]], { expect: { lastSent: "drag", drags: 1 } }],
  // After a drag the next click elsewhere still works (Windows' drag loop takes the mouseup).
  ["drag-then-click-en-light", "en", "light", [["type", "report"], ["drag", "#r1"], ["wait", 50], ["click", "#set-btn"]], { expect: { drags: 1, settings: true } }],
  // A program the search names: a button above the results; a click asks for it by its place.
  ["apps-ar-light", "ar", "light", [["type", "كروم"], ["wait", 100], ["click", "#apps [data-app=\"0\"]"], ["wait", 50]], { expect: { apps: 1, lastSent: "app" } }],
  // «فتح قائمة الخيارات…»: the program's Options menu (tools, support, updates).
  ["options-menu-en-dark", "en", "dark", [["key", "Control+,"], ["click", "#options-menu"], ["wait", 50]], { expect: { settings: true, lastSent: "options_menu" } }],
  // «الإبلاغ عن مشكلة…»: the program's report window (it shows what would be shared first).
  ["report-problem-ar-light", "ar", "light", [["key", "Control+,"], ["click", "#report-problem"], ["wait", 50]], { expect: { settings: true, lastSent: "report_problem" } }],
  // A name split into base and extension keeps its matches marked, in the extension too.
  ["names-split-ar-light", "ar", "light", [["type", "xlsx"]], { expect: { marks: 2, fnames: 2 } }],
  // «…بتاريخ 2026-11-12 … هاتف 0770-123-4567.txt»: the date and the phone read
  // as typed, in the list, its folder, the preview and the status bar's
  // file count, a match inside the phone marked.
  ["digit-runs-ar-light", "ar", "light", [["type", "123-45"]], { expect: { rows: 1, marks: 1, runs: 7 } }],
  // Ctrl+Alt+G in a Save/Open dialog: a line says what Enter does; Enter moves the dialog to the
  // selected result's folder (not open it), Esc cancels; the result menu has the move first.
  ["qswitch-ar-light", "ar", "light", [["eval", QS_ON.ar], ["type", "تقرير"], ["key", "ArrowDown"]], { expect: { qswitch: true } }],
  ["qswitch-enter-en-light", "en", "light", [["eval", QS_ON.en], ["type", "report"], ["key", "Enter"], ["wait", 50]], { expect: { lastSent: "switch_to", opens: 0, qswitch: false } }],
  ["qswitch-esc-en-dark", "en", "dark", [["eval", QS_ON.en], ["type", "report"], ["key", "Escape"], ["wait", 50]], { expect: { lastSent: "switch_cancel", qswitch: false, q: "report" } }],
  ["qswitch-ctx-ar-dark", "ar", "dark", [["eval", QS_ON.ar], ["type", "تقرير"], ["rclick", "#r1"]], { expect: { menu: true, ctxFirst: "فتح مجلدها في نافذة الحفظ/الفتح" } }],
  // The quick popup's suggestions carry over: inside documents runs a document search, all types clears the type.
  ["handover-content-ar-light", "ar", "light", [["eval", "window.__instafind.recv({ type: 'query', q: 'عقد الإيجار', content: true, all_kinds: false })"], ["wait", 300]], { expect: { content: true, rows: 3, inside: true } }],
  ["handover-kinds-en-light", "en", "light", [["chip", "kind", 1], ["eval", "window.__instafind.recv({ type: 'query', q: 'report', content: false, all_kinds: true })"], ["wait", 150]], { expect: { chipsOn: 0, inside: false } }],
  // Windows' "Open with" and Properties in the result menu (Properties last, as in Explorer; Alt+Enter too).
  ["ctx-open-with-ar-light", "ar", "light", [["type", "تقرير"], ["rclick", "#r1"], ["click", "#ctx [data-op=open_with]"], ["wait", 50]], { expect: { lastSent: "open_with" } }],
  ["ctx-properties-en-dark", "en", "dark", [["type", "report"], ["rclick", "#r1"], ["click", "#ctx [role=menuitem]:last-child"], ["wait", 10], ["wait", 50]], { expect: { lastSent: "properties" } }],
  ["alt-enter-en-light", "en", "light", [["type", "report"], ["key", "Alt+Enter"], ["wait", 50]], { expect: { lastSent: "properties", opens: 0 } }],
  // "Clear history and recent files" (the classic label): on with opened files and no searches; it clears the files too.
  ["history-clear-ar-light", "ar", "light", [["click", "#hist-btn"]], { noSearches: true, expect: { menu: true, clearItem: "مسح السجل والملفات الأخيرة|on" } }],
  ["history-cleared-en-light", "en", "light", [["click", "#hist-btn"], ["click", "#hist-clear"], ["wait", 50], ["click", "#hist-btn"]], { expect: { lastSent: "clear_recent", recentRows: 0, clearItem: "Clear history and recent files|off" } }],
  ["qswitch-ctx-click-en-light", "en", "light", [["eval", QS_ON.en], ["type", "report"], ["rclick", "#r1"], ["click", "#ctx [role=menuitem]:first-child"], ["wait", 50]], { expect: { lastSent: "switch_to", qswitch: false } }],
];

(async () => {
  const browser = await chromium.launch(exe ? { executablePath: exe } : {});
  const errors = [];
  for (const [name, lang, theme, steps, opts = {}] of SHOTS) {
    const ctx = await browser.newContext({ viewport: { width: opts.width || 1280, height: opts.height || 800 }, deviceScaleFactor: 2, colorScheme: theme, locale: lang === "ar" ? "ar" : "en-US" });
    const pg = await ctx.newPage();
    const thumbAsks = [];
    pg.on("pageerror", (e) => errors.push(`${name}: ${e.message}`));
    // (A thumbnail the program has none for is a 404, as it should be.)
    pg.on("console", (m) => (m.type() === "error" || /Content Security Policy/.test(m.text())) && !(/404/.test(m.text()) && /\/thumb\/[0-9a-f]{32}$/.test((m.location() || {}).url || "")) && errors.push(`${name}: console: ${m.text()}`));
    await pg.route("**/*", (route) => {
      const url = new URL(route.request().url());
      if (url.origin !== ORIGIN) { errors.push(`${name}: request outside the program: ${url}`); return route.abort(); }
      const headers = { "Content-Security-Policy": CSP, "X-Content-Type-Options": "nosniff" };
      if (url.pathname === "/" || url.pathname === "/index.html") return route.fulfill({ status: 200, contentType: "text/html; charset=utf-8", headers, body: page(lang) });
      // Thumbnails: only /thumb/<32 hex>, as assets::thumb_key; a picture for keys of even rows.
      if (url.pathname.startsWith("/thumb")) {
        thumbAsks.push(url.pathname);
        if (!/^\/thumb\/[0-9a-f]{32}$/.test(url.pathname) || url.search) errors.push(`${name}: odd thumbnail request ${url.pathname}${url.search}`);
        return url.pathname.startsWith("/thumb/a") ? route.fulfill({ status: 200, contentType: "image/png", headers, body: TEST_PNG }) : route.fulfill({ status: 404, headers, body: "" });
      }
      const file = path.join(ASSETS, url.pathname);
      if (!file.startsWith(ASSETS) || !fs.existsSync(file)) return route.fulfill({ status: 404, headers, body: "" });
      route.fulfill({ status: 200, contentType: MIME[path.extname(file)] || "application/octet-stream", headers, body: fs.readFileSync(file) });
    });
    await pg.addInitScript(fakeProgram, { lang, indexing: !!opts.indexing, keepRunning: !!opts.keepRunning, switched: !!opts.switched, noSearches: !!opts.noSearches });
    await pg.goto(ORIGIN + "/");
    await pg.waitForFunction(() => document.querySelector("#menu-kind") && document.querySelector("#menu-kind").children.length > 0);
    for (const [what, a, b] of steps) {
      if (what === "type") { await pg.fill("#q", a); await pg.waitForTimeout(150); }
      else if (what === "key") { await pg.keyboard.press(a); await pg.waitForTimeout(60); }
      else if (what === "click") { await pg.click(a); await pg.waitForTimeout(120); }
      else if (what === "rclick") { await pg.click(a, { button: "right" }); await pg.waitForTimeout(120); }
      else if (what === "fill") { await pg.fill(a, b); }
      else if (what === "chip") { await pg.click(`#chip-${a}`); await pg.click(`#menu-${a} [data-k="${b}"]`); await pg.waitForTimeout(120); }
      else if (what === "wait") await pg.waitForTimeout(a);
      else if (what === "eval") await pg.evaluate(a);
      else if (what === "drag") {
        const b = await pg.locator(a).boundingBox();
        await pg.mouse.move(b.x + b.width / 2, b.y + b.height / 2);
        await pg.mouse.down();
        await pg.mouse.move(b.x + b.width / 2 + 30, b.y + b.height / 2 + 12, { steps: 5 });
        await pg.mouse.up();
        await pg.waitForTimeout(60);
      }
    }
    await pg.evaluate(() => document.fonts.ready);
    await pg.waitForTimeout(100);
    const check = await pg.evaluate((lang) => {
      const bad = [...window.__fakeErrors];
      const visible = (el) => el && !el.closest("[hidden]") && el.getClientRects().length > 0;
      if (document.documentElement.scrollWidth > innerWidth) bad.push("horizontal overflow");
      if ([...document.fonts].filter((f) => f.status === "loaded").length < 2) bad.push("fonts not loaded");
      if (document.documentElement.dir !== (lang === "ar" ? "rtl" : "ltr")) bad.push("page direction");
      if (getComputedStyle(document.body).direction !== (lang === "ar" ? "rtl" : "ltr")) bad.push("body direction");
      // Every path: left to right, its folder names placed left to right in order.
      for (const p of document.querySelectorAll(".path")) {
        if (getComputedStyle(p).direction !== "ltr") bad.push("path not LTR: " + p.textContent);
        const xs = [...p.querySelectorAll(":scope > bdi")].filter((b) => b.getClientRects().length).map((b) => b.getClientRects()[0]);
        for (let i = 1; i < xs.length; i++) if (xs[i].top === xs[i - 1].top && xs[i].left < xs[i - 1].left) bad.push("path segments out of order: " + p.textContent);
      }
      // The empty search box's hint reads in the page's direction («ابحث باسم الملف…», its ellipsis at the end).
      const box = document.querySelector("#q");
      if (!box.value && getComputedStyle(box).direction !== (lang === "ar" ? "rtl" : "ltr")) bad.push("search hint not in the page's direction");
      // Names, snippets and matching lines pick their own direction.
      for (const el of document.querySelectorAll(".pv-name .t, .row .n3 .sn, .lines .lt, .field input")) if (visible(el) && getComputedStyle(el).unicodeBidi !== "plaintext" && el.tagName !== "BDI" && !(el.classList.contains("fname") && el.dir) && !el.matches(":placeholder-shown")) bad.push("text without its own direction: " + el.className);
      // Phone numbers, dates and versions (app.js DIGIT_RUN) read as typed:
      // each character of a run to the right of the one before it.
      let runs = 0;
      for (const el of document.querySelectorAll(".run")) {
        if (!visible(el)) continue;
        runs++;
        const rs = [];
        const walk = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
        while (walk.nextNode()) for (let i = 0; i < walk.currentNode.length; i++) { const r = document.createRange(); r.setStart(walk.currentNode, i); r.setEnd(walk.currentNode, i + 1); rs.push(r.getBoundingClientRect()); }
        if (rs.some((r) => Math.abs(r.top - rs[0].top) > 2)) continue; // wrapped: no order on one line
        if (rs.some((r, i) => i && !(r.left > rs[i - 1].left))) bad.push("digit run out of order: " + el.textContent);
      }
      // File names: the extension (".xlsx") left to right, at the end of the
      // name as it reads: on the left of an Arabic name, on the right of an
      // English one.
      let fnames = 0;
      for (const el of document.querySelectorAll(".fname")) {
        if (!visible(el)) continue;
        fnames++;
        const base = el.querySelector(":scope > .base"), ext = el.querySelector(":scope > .ext");
        if (!base || !ext || !ext.textContent.startsWith(".") || getComputedStyle(ext).direction !== "ltr") { bad.push("file name not split: " + el.textContent); continue; }
        if (base.getClientRects().length > 1) continue; // wrapped (a tile): no order on one line
        const b = base.getBoundingClientRect(), x = ext.getBoundingClientRect();
        if (b.width === 0 || Math.abs(b.bottom - x.bottom) > 2) continue; // wrapped: no order on one line
        const rtl = /^[^\p{L}]*[\u0590-\u08FF\uFB1D-\uFDFF\uFE70-\uFEFF]/u.test(base.textContent);
        if (rtl ? x.right > b.left + 1 : x.left < b.right - 1) bad.push(`extension on the wrong side (${rtl ? "rtl" : "ltr"}): ${el.textContent}`);
        // A right-to-left base and its extension are not glued together.
        if (rtl && b.left - x.right < 2) bad.push(`extension glued to the name: ${el.textContent}`);
      }
      // Strings that went missing.
      const text = document.body.innerText;
      for (const w of ["undefined", "NaN", "[object", "null"]) if (text.includes(w)) bad.push(`"${w}" on screen`);
      for (const el of document.querySelectorAll("[data-i18n]")) if (!el.textContent.trim()) bad.push("no text for " + el.dataset.i18n);
      for (const el of document.querySelectorAll("button")) if (visible(el) && !el.textContent.trim() && !el.getAttribute("aria-label")) bad.push("button without a name: " + (el.id || el.className));
      // Menus and dialogs fully on screen.
      const shown = [...document.querySelectorAll(".menu, .dlg")].filter(visible);
      for (const el of shown) {
        const r = el.getBoundingClientRect();
        if (r.left < 0 || r.top < 0 || r.right > innerWidth + 0.5 || r.bottom > innerHeight + 0.5) bad.push(`${el.id || el.className} off screen`);
        if (el.classList.contains("menu") && r.width > innerWidth / 2) bad.push(`${el.id || el.className} stretched to ${Math.round(r.width)}px`);
      }
      const view = ["view-results", "view-empty", "view-nores", "view-settings"].find((id) => !document.getElementById(id).hidden);
      return {
        bad, view, dir: document.documentElement.dir, rows: document.querySelectorAll("#list .row").length,
        active: document.querySelector("#q").getAttribute("aria-activedescendant"),
        menu: shown.some((el) => el.classList.contains("menu")), dialog: shown.some((el) => el.classList.contains("dlg")),
        content: document.querySelectorAll("#list .row .n3").length > 0, stop: visible(document.getElementById("stop")),
        chipsOn: document.querySelectorAll(".chip.on").length, settings: view === "view-settings",
        names: [...document.querySelectorAll("#list .row .n1 .t")].map((e) => e.textContent), gone: document.querySelectorAll("#list .row.gone").length,
        fnames: document.querySelectorAll("#list .row .fname").length, runs, marks: document.querySelectorAll("#list .row .n1 mark").length,
        sent: window.__sent.map((m) => m.op), recycles: window.__sent.filter((m) => m.op === "recycle").length,
        stale: window.__stale.length, q: document.querySelector("#q").value, focus: document.activeElement && document.activeElement.id,
        lastSent: window.__sent.length ? window.__sent[window.__sent.length - 1].op : null,
        drags: window.__sent.filter((m) => m.op === "drag").length,
        apps: document.querySelectorAll("#apps:not([hidden]) button").length,
        opens: window.__sent.filter((m) => m.op === "open").length,
        stillIndexing: document.getElementById("view-nores").classList.contains("indexing"),
        switched: !document.getElementById("switched").hidden,
        warn: document.getElementById("status-text").classList.contains("warn"),
        qAlign: getComputedStyle(document.querySelector("#q")).textAlign,
        compact: document.body.classList.contains("compact"),
        grid: document.querySelectorAll("#list .row.tile").length > 0,
        pics: document.querySelectorAll("#list .row.tile.pic").length > 0,
        downByCols: (() => { const l = document.getElementById("list"); const cols = getComputedStyle(l).gridTemplateColumns.split(" ").length; return cols > 1 && l.getAttribute("aria-activedescendant") === "r" + cols; })(),
        tagline: visible(document.querySelector("#view-nores .tagline")),
        dateW: document.body.style.getPropertyValue("--w-date"),
        linesTop: (() => { const l = document.querySelector(".preview .pv-lines, .preview .snip"); return !!l && l.getBoundingClientRect().top < innerHeight * 0.75; })(),
        zoom: document.documentElement.style.zoom || "1",
        pathPx: [...document.querySelectorAll("#list .row .n2")].every((p) => parseFloat(getComputedStyle(p).fontSize) >= 13.5),
        termsAr: document.getElementById("cols").textContent.includes("تاريخ التعديل") && document.getElementById("status-text").textContent.startsWith("وجدت"),
        dateTime: /[0-9٠-٩]{1,2}:[0-9٠-٩]{2}/.test([...document.querySelectorAll(".facts dd")].map((d) => d.textContent).join(" ")),
        actbar: visible(document.getElementById("actbar")) ? document.querySelectorAll("#actbar .btn").length : 0,
        labels: [...document.querySelectorAll(".hbtn .hlbl")].every((l) => visible(l) && l.textContent.trim()),
        rowH: [...document.querySelectorAll("#list .row")].every((r) => r.getBoundingClientRect().height <= 36),
        preview: visible(document.getElementById("preview")),
        bannerSlim: document.getElementById("banner").classList.contains("slim") && document.getElementById("banner").getBoundingClientRect().height <= 48,
        dlgName: !!document.querySelector("#dlg-h .fname"),
        qswitch: visible(document.getElementById("qswitch")),
        clearItem: (() => { const it = document.getElementById("hist-clear"); return it ? `${it.textContent}|${it.getAttribute("aria-disabled") === "true" ? "off" : "on"}` : null; })(),
        recentRows: document.querySelectorAll("#recent .row").length,
        inside: document.getElementById("chip-content").getAttribute("aria-pressed") === "true",
        ctxFirst: (() => { const it = document.querySelector("#ctx:not([hidden]) [role=menuitem] span"); return it ? it.textContent : null; })(),
      };
    }, lang);
    for (const b of check.bad) errors.push(`${name}: ${b}`);
    const want = opts.expect || {};
    if (check.stale && !("stale" in want)) errors.push(`${name}: a request named an older list: ${check.stale}`);
    for (const [k, v] of Object.entries(want)) {
      const ok = k === "renamed" ? check.names.includes(v) : check[k] === v;
      if (!ok) errors.push(`${name}: expected ${k} = ${JSON.stringify(v)}, got ${JSON.stringify(k === "renamed" ? check.names : check[k])}`);
    }
    console.log(name, JSON.stringify({ view: check.view, dir: check.dir, rows: check.rows, active: check.active, menu: check.menu, dialog: check.dialog, chipsOn: check.chipsOn }));
    await pg.screenshot({ path: path.join(out, `${name}.png`) });
    await ctx.close();
  }
  await browser.close();
  if (errors.length) { console.error(errors.join("\n")); process.exit(1); }
  console.log(`rendered ${SHOTS.length} screens to ${out}`);
})();
