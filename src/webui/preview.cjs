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
// marked, and that every message the page
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
    rename: ["list", "seq", "ver", "id", "name"], recycle: ["list", "seq", "ver", "id"], scope: ["list", "seq", "ver", "id"], pick_folder: [], all_drives: [], saved: ["id"],
    save_search: ["q", "kind", "content"], clear_recent: [], clear_saved: [], set: ["key", "on"], lang: ["lang"], restart: [], reindex: [], web: ["q"], hide: [], classic_look: [], options_menu: [], drag: ["list", "seq", "ver", "id"], app: ["seq", "id"],
  };
  const REQUIRED = { search: ["seq", "q"], sort: ["seq", "sort"], set: ["key", "on"], rename: ["list", "seq", "ver", "id", "name"] };
  // Requests about rows, and the list version they must name (a new one for every list sent and every new order).
  const ROW_OPS = ["more", "matches", "open", "reveal", "copy", "copy_file", "copy_all", "rename", "recycle", "scope", "drag"];
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
    ? { type: "status", state: "names", files: 36000, text: ar ? "جارِ الفهرسة… 36,000 ملف حتى الآن" : "Indexing… 36,000 files so far" }
    : { type: "status", state: "ready", files: 1204331, text: null };
  const settings = { type: "settings", lang: cfg.lang, next_lang: cfg.lang, digits: false, autostart: true, content_index: true, auto_index: true, hide_system: true, hotkey_quick: true, floating: true, tray_icon: true, version: "0.9.1" };
  const history = {
    type: "history",
    recent: ar ? ["تقرير المبيعات", "عقد الإيجار", "فاتورة الكهرباء .pdf", "صور العيد"] : ["sales report", "lease agreement", "electricity bill .pdf", "holiday photos"],
    saved: [{ id: 0, name: ar ? "عقود العمل" : "Work contracts", content: true, scoped: true }, { id: 1, name: ar ? "تقارير هذا الشهر" : "Reports this month", content: false, scoped: false }],
  };
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
        if (m.op !== "more" && m.op !== "matches") { window.__stale.push(m.op); send({ type: "notice", text: ar ? "تغيّرت القائمة للتو، فلم يُنفَّذ شيء. حاول مرة أخرى." : "The list just changed, so nothing was done. Try again." }); }
        return;
      }
    }
    if (m.op === "hello") {
      send({ type: "hello", kinds: KINDS, sizes: SIZES, dates: DATES, kind: 0, sort: "best", desc: false, digits: false, version: "0.9.1" });
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
        send({ type: "progress", seq: m.seq, total: found.length, done: false, text: ar ? "جارِ البحث داخل المستندات… تم فحص 1,840 من 4,210 — وجدت 3" : "Searching inside documents… checked 1,840 of 4,210 — found 3" });
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
    } else if (m.op === "save_search" || m.op === "clear_recent" || m.op === "clear_saved") {
      if (m.op === "clear_recent") history.recent = [];
      send({ ...history });
    }
  } } });
  // The name search run again in the background (the index changed), in another order.
  window.__rerun = () => { shown.list = [...shown.list].reverse(); shown.ver = nextVer(); send(results({ seq: shown.seq, kind: 0 }, {})); };
}

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
  ["matches-ar-light", "ar", "light", [["click", "#chip-content"], ["type", "عقد الإيجار"], ["key", "Enter"], ["wait", 300], ["key", "F4"], ["wait", 150]], { expect: { dialog: true } }],
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
  ["recycle-cancel-ar-dark", "ar", "dark", [["type", "تقرير"], ["rclick", "#r2"], ["click", "#ctx [role=menuitem]:last-child"], ["wait", 50], ["key", "Escape"], ["wait", 50]], { expect: { dialog: false, recycles: 0, gone: 0, focus: "list" } }],
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
  // «العودة إلى الشكل القديم»: one typed request.
  ["classic-look-ar-light", "ar", "light", [["click", "#set-btn"], ["click", "#classic-look"], ["wait", 50]], { expect: { settings: true, lastSent: "classic_look" } }],
  // Dragging a row out asks the program to drag that file (once), naming its list version.
  ["drag-en-light", "en", "light", [["type", "report"], ["drag", "#r1"]], { expect: { drags: 1, dialog: false } }],
  ["drag-recent-ar-light", "ar", "light", [["drag", "#rr0"]], { expect: { lastSent: "drag", drags: 1 } }],
  // A program the search names: a button above the results; a click asks for it by its place.
  ["apps-ar-light", "ar", "light", [["type", "كروم"], ["wait", 100], ["click", "#apps [data-app=\"0\"]"], ["wait", 50]], { expect: { apps: 1, lastSent: "app" } }],
  // «فتح قائمة الخيارات…»: the program's Options menu (tools, support, updates).
  ["options-menu-en-dark", "en", "dark", [["key", "Control+,"], ["click", "#options-menu"], ["wait", 50]], { expect: { settings: true, lastSent: "options_menu" } }],
  // A name split into base and extension keeps its matches marked, in the extension too.
  ["names-split-ar-light", "ar", "light", [["type", "xlsx"]], { expect: { marks: 2, fnames: 2 } }],
];

(async () => {
  const browser = await chromium.launch(exe ? { executablePath: exe } : {});
  const errors = [];
  for (const [name, lang, theme, steps, opts = {}] of SHOTS) {
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2, colorScheme: theme, locale: lang === "ar" ? "ar" : "en-US" });
    const pg = await ctx.newPage();
    pg.on("pageerror", (e) => errors.push(`${name}: ${e.message}`));
    pg.on("console", (m) => (m.type() === "error" || /Content Security Policy/.test(m.text())) && errors.push(`${name}: console: ${m.text()}`));
    await pg.route("**/*", (route) => {
      const url = new URL(route.request().url());
      if (url.origin !== ORIGIN) { errors.push(`${name}: request outside the program: ${url}`); return route.abort(); }
      const headers = { "Content-Security-Policy": CSP, "X-Content-Type-Options": "nosniff" };
      if (url.pathname === "/" || url.pathname === "/index.html") return route.fulfill({ status: 200, contentType: "text/html; charset=utf-8", headers, body: page(lang) });
      const file = path.join(ASSETS, url.pathname);
      if (!file.startsWith(ASSETS) || !fs.existsSync(file)) return route.fulfill({ status: 404, headers, body: "" });
      route.fulfill({ status: 200, contentType: MIME[path.extname(file)] || "application/octet-stream", headers, body: fs.readFileSync(file) });
    });
    await pg.addInitScript(fakeProgram, { lang, indexing: !!opts.indexing, keepRunning: !!opts.keepRunning });
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
      // File names: the extension (".xlsx") left to right, at the end of the
      // name as it reads: on the left of an Arabic name, on the right of an
      // English one.
      let fnames = 0;
      for (const el of document.querySelectorAll(".fname")) {
        if (!visible(el)) continue;
        fnames++;
        const base = el.querySelector(":scope > .base"), ext = el.querySelector(":scope > .ext");
        if (!base || !ext || !ext.textContent.startsWith(".") || getComputedStyle(ext).direction !== "ltr") { bad.push("file name not split: " + el.textContent); continue; }
        const b = base.getBoundingClientRect(), x = ext.getBoundingClientRect();
        if (b.width === 0 || Math.abs(b.bottom - x.bottom) > 2) continue; // wrapped: no order on one line
        const rtl = /^[^\p{L}]*[\u0590-\u08FF\uFB1D-\uFDFF\uFE70-\uFEFF]/u.test(base.textContent);
        if (rtl ? x.right > b.left + 1 : x.left < b.right - 1) bad.push(`extension on the wrong side (${rtl ? "rtl" : "ltr"}): ${el.textContent}`);
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
        fnames: document.querySelectorAll("#list .row .fname").length, marks: document.querySelectorAll("#list .row .n1 mark").length,
        sent: window.__sent.map((m) => m.op), recycles: window.__sent.filter((m) => m.op === "recycle").length,
        stale: window.__stale.length, q: document.querySelector("#q").value, focus: document.activeElement && document.activeElement.id,
        lastSent: window.__sent.length ? window.__sent[window.__sent.length - 1].op : null,
        drags: window.__sent.filter((m) => m.op === "drag").length,
        apps: document.querySelectorAll("#apps:not([hidden]) button").length,
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
