// Renders the new main window's page (src/webui/assets) in headless Chromium
// with a fake program behind window.ipc and synthetic files, checks it, and
// saves screenshots. Development tool only: not part of the exe or of CI.
//
//   NODE_PATH=<folder with playwright> node src/webui/preview.cjs <outDir> [chromium path]
//
// The page is served exactly as the exe serves it (same URL, same CSP
// header), so a CSP violation, a request to anywhere else, a script error or
// a horizontal overflow fails the run. It also checks that every path is laid
// out left to right with its folder names in order, in Arabic too.
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
  const files = (ar ? [
    [`${U}\\Documents\\العمل\\المالية`, "تقرير المبيعات 2026.xlsx", 48 * 1024, 7 * 60],
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
  const KINDS = ar ? ["كل الأنواع", "مستندات", "صور", "صوت", "فيديو", "مضغوطات", "برامج", "مجلدات"] : ["All types", "Documents", "Pictures", "Audio", "Video", "Archives", "Programs", "Folders"];
  const DOCS = ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt"];
  const send = (m) => setTimeout(() => window.__instafind.recv(m), 0);
  const row = (f, id, q) => {
    const i = q ? f.name.toLowerCase().indexOf(q.toLowerCase()) : -1;
    const name = i < 0 ? [[f.name, false]] : [[f.name.slice(0, i), false], [f.name.slice(i, i + q.length), true], [f.name.slice(i + q.length), false]].filter(([t]) => t);
    return { id, name, dir: f.dir, size: f.size, mtime: f.mtime, ext: f.ext, folder: f.folder };
  };
  const total = 312000 + 4331;
  const status = cfg.indexing
    ? { type: "status", state: "names", files: 36000, text: ar ? "جارِ الفهرسة… 36,000 ملف حتى الآن" : "Indexing… 36,000 files so far" }
    : { type: "status", state: "ready", files: 1204331, text: null };
  Object.defineProperty(window, "ipc", { value: { postMessage(s) {
    const m = JSON.parse(s);
    if (m.t !== "00112233445566778899aabbccddeeff") throw new Error("no token");
    if (m.op === "hello") {
      send({ type: "hello", kinds: KINDS, digits: false, version: "0.9.1" });
      send(status);
      send({ type: "recent", rows: files.slice(0, 6).map((f, i) => row(f, i, "")) });
    } else if (m.op === "search") {
      const words = m.q.trim().split(/\s+/);
      const term = words[0] === "تقرير" ? "تقرير" : words[0];
      let hits = files.filter((f) => f.name.toLowerCase().includes(term.toLowerCase()));
      if (m.kind === 1) hits = hits.filter((f) => DOCS.includes(f.ext));
      if (cfg.indexing) hits = hits.slice(0, 4);
      const n = cfg.indexing ? hits.length : hits.length ? Math.max(hits.length, 312) : 0;
      const sugg = hits.length ? [] : [
        { kind: "query", label: ar ? "هل تقصد «عقد الإيجار 2023»؟" : "Did you mean “lease 2023”?", q: ar ? "عقد الإيجار 2023" : "lease 2023" },
        ...(m.kind ? [{ kind: "all_kinds", label: ar ? "ابحث في كل أنواع الملفات" : "Search all file types", q: "" }] : []),
        { kind: "web", label: ar ? "ابحث في الويب" : "Search the web", q: "" },
      ];
      send({ type: "results", seq: m.seq, total: n, ms: 18, rows: hits.map((f, i) => row(f, i, term)), note: null, suggestions: sugg, files: status.files, kind_label: m.kind ? KINDS[m.kind] : "" });
    }
  } } });
}

const SHOTS = [
  // name, lang, theme, query, kind, indexing
  ["main-ar-light", "ar", "light", "تقرير", 0, false],
  ["main-en-dark", "en", "dark", "report", 0, false],
  ["empty-ar-light", "ar", "light", "", 0, false],
  ["nores-ar-dark", "ar", "dark", "عقد الايجار 2024", 1, false],
  ["indexing-ar-light", "ar", "light", "تقرير", 0, true],
  ["nores-en-light", "en", "light", "lease 2024", 0, false],
];

(async () => {
  const browser = await chromium.launch(exe ? { executablePath: exe } : {});
  const errors = [];
  for (const [name, lang, theme, query, kind, indexing] of SHOTS) {
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1, colorScheme: theme, locale: lang === "ar" ? "ar" : "en-US" });
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
    await pg.addInitScript(fakeProgram, { lang, indexing });
    await pg.goto(ORIGIN + "/");
    await pg.waitForFunction(() => document.querySelector("#kind-menu").children.length > 0);
    if (kind) { await pg.click("#kind"); await pg.click(`#kind-menu [data-k="${kind}"]`); }
    if (query) { await pg.fill("#q", query); await pg.waitForTimeout(150); }
    if (name.startsWith("main")) { await pg.keyboard.press("ArrowDown"); }
    await pg.evaluate(() => document.fonts.ready);
    await pg.waitForTimeout(100);
    const check = await pg.evaluate(() => {
      const bad = [];
      if (document.documentElement.scrollWidth > innerWidth) bad.push("horizontal overflow");
      if ([...document.fonts].filter((f) => f.status === "loaded").length < 2) bad.push("fonts not loaded");
      // Every path: left to right, its folder names placed left to right in order.
      for (const p of document.querySelectorAll(".path")) {
        if (getComputedStyle(p).direction !== "ltr") bad.push("path not LTR: " + p.textContent);
        const xs = [...p.querySelectorAll("bdi")].filter((b) => b.getClientRects().length).map((b) => b.getClientRects()[0]);
        for (let i = 1; i < xs.length; i++) if (xs[i].top === xs[i - 1].top && xs[i].left < xs[i - 1].left) bad.push("path segments out of order: " + p.textContent);
      }
      const view = ["view-results", "view-empty", "view-nores"].find((id) => !document.getElementById(id).hidden);
      return { bad, view, dir: document.documentElement.dir, rows: document.querySelectorAll("#list .row").length, active: document.querySelector("#q").getAttribute("aria-activedescendant") };
    });
    for (const b of check.bad) errors.push(`${name}: ${b}`);
    console.log(name, JSON.stringify({ view: check.view, dir: check.dir, rows: check.rows, active: check.active }));
    await pg.screenshot({ path: path.join(out, `${name}.png`) });
    await ctx.close();
  }
  await browser.close();
  if (errors.length) { console.error(errors.join("\n")); process.exit(1); }
  console.log(`rendered ${SHOTS.length} screens to ${out}`);
})();
