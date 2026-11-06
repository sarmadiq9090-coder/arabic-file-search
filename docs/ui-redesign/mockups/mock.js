// InstaFind UI mockup (design only). All file names, folders and people are synthetic.
// URL: index.html?screen=main|quick|settings|first|empty|nores|indexing&lang=ar|en&theme=light|dark[&shot=1]
"use strict";

const P = new URLSearchParams(location.search);
const S = {
  screen: P.get("screen") || "main",
  lang: P.get("lang") === "en" ? "en" : "ar",
  theme: P.get("theme") === "dark" ? "dark" : "light",
  shot: P.get("shot") === "1",
  q: null, sel: 1, inside: true, kind: null,
};

// ---------- icons ----------
const ICON = {
  search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.6-3.6"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  sliders: '<path d="M4 7h9M17 7h3M4 17h3M11 17h9"/><circle cx="15" cy="7" r="2"/><circle cx="9" cy="17" r="2"/>',
  file: '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/>',
  doctext: '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5M9 13h6M9 17h4"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
  cal: '<rect x="3" y="5" width="18" height="16" rx="2"/><path d="M3 10h18M8 3v4M16 3v4"/>',
  size: '<path d="M4 19h16M7 19v-8M12 19V6M17 19v-5"/>',
  x: '<path d="M6 6l12 12M18 6 6 18"/>',
  down: '<path d="m6 9 6 6 6-6"/>',
  open: '<path d="M14 4h6v6M20 4l-9 9"/><path d="M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5"/>',
  copy: '<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h3"/>',
  dots: '<circle cx="5" cy="12" r="1.3"/><circle cx="12" cy="12" r="1.3"/><circle cx="19" cy="12" r="1.3"/>',
  star: '<path d="m12 3 2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1-4.4-4.3 6.1-.9z"/>',
  drive: '<rect x="3" y="13" width="18" height="7" rx="2"/><path d="M3 15 6 5h12l3 10"/><path d="M17 16.5h.01"/>',
  shield: '<path d="M12 3l7 3v6c0 4.5-3 7.5-7 9-4-1.5-7-4.5-7-9V6z"/><path d="m9 12 2 2 4-4"/>',
  globe: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.5 2.7 3.8 5.7 3.8 9s-1.3 6.3-3.8 9c-2.5-2.7-3.8-5.7-3.8-9S9.5 5.7 12 3z"/>',
  bulb: '<path d="M9 18h6M10 21h4M12 3a6 6 0 0 0-4 10.5c.7.7 1 1.5 1 2.5h6c0-1 .3-1.8 1-2.5A6 6 0 0 0 12 3z"/>',
  pause: '<path d="M9 5v14M15 5v14"/>',
  filter: '<path d="M4 5h16l-6 8v6l-4-2v-4z"/>',
  general: '<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M3 9h18"/>',
  bolt: '<path d="M13 3 5 14h6l-1 7 8-11h-6z"/>',
  palette: '<path d="M12 3a9 9 0 1 0 0 18c1.2 0 1.8-.8 1.8-1.7 0-1.3-1-1.6-1-2.6 0-.9.7-1.6 1.7-1.6H17a4 4 0 0 0 4-4C21 6.6 17 3 12 3z"/><circle cx="7.5" cy="11" r="1"/><circle cx="10.5" cy="7.5" r="1"/><circle cx="15" cy="7.5" r="1"/>',
  refresh: '<path d="M20 11a8 8 0 1 0-2.3 5.7"/><path d="M20 4v7h-7"/>',
  wrench: '<path d="M14.7 6.3a4 4 0 0 0 5 5L21 13l-8 8-3-3 8-8"/><path d="M14.7 6.3 13 4.6 4.6 13a2 2 0 0 0 0 2.8l.6.6"/>',
  min: '<path d="M5 12h14"/>', max: '<rect x="5" y="5" width="14" height="14" rx="1.5"/>', close: '<path d="M6 6l12 12M18 6 6 18"/>',
  keyboard: '<rect x="2.5" y="6" width="19" height="12" rx="2"/><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10"/>',
  calc: '<rect x="5" y="3" width="14" height="18" rx="2"/><path d="M8 7h8M8 11h.01M12 11h.01M16 11h.01M8 15h.01M12 15h.01M16 15h.01M8 18h.01M12 18h.01"/>',
};
const ic = (n, cls = "i") => `<svg class="${cls}" viewBox="0 0 24 24" aria-hidden="true">${ICON[n]}</svg>`;

// New mark (proposal): forest tile, sky lens, lime nuqta = "the thing you were looking for". Never mirrored.
const LOGO = '<svg class="logo" viewBox="0 0 32 32" aria-hidden="true"><rect width="32" height="32" rx="8" fill="#123B2A"/><circle cx="14" cy="14" r="7" fill="none" stroke="#78CFFF" stroke-width="3"/><path d="M19.4 19.4 24.5 24.5" stroke="#78CFFF" stroke-width="3.4" stroke-linecap="round"/><rect x="11.9" y="11.9" width="4.2" height="4.2" rx=".6" transform="rotate(45 14 14)" fill="#B9F34A"/></svg>';

const FT = {
  pdf: ["#D93F3F", "PDF"], xls: ["#1E8E4E", "XLS"], doc: ["#2F64D6", "DOC"], ppt: ["#D9682B", "PPT"],
  txt: ["#6B7F74", "TXT"], zip: ["#9A7A12", "ZIP"], img: ["#8A55C9", "IMG"], aud: ["#C2417F", "M4A"], exe: ["#123B2A", "EXE"],
};
function ft(type) {
  if (type === "dir") return '<svg class="ft" viewBox="0 0 30 34" aria-hidden="true"><path d="M2 9a3 3 0 0 1 3-3h7l3 3h10a3 3 0 0 1 3 3v13a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3z" fill="#F2B544"/><path d="M2 13h26v12a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3z" fill="#F7C95F"/></svg>';
  const [c, t] = FT[type] || FT.txt;
  return `<svg class="ft" viewBox="0 0 30 34" aria-hidden="true"><path d="M5 1h14l8 8v21a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3V4a3 3 0 0 1 3-3z" fill="${c}"/><path d="M19 1v5a3 3 0 0 0 3 3h5z" fill="#fff" fill-opacity=".45"/><text x="14.5" y="25" text-anchor="middle" font-family="IF Sans, sans-serif" font-size="7.5" font-weight="600" fill="#fff">${t}</text></svg>`;
}

// ---------- helpers ----------
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
function hl(text, q) {
  if (!q) return esc(text);
  const i = text.toLowerCase().indexOf(q.toLowerCase());
  if (i < 0) return esc(text);
  return esc(text.slice(0, i)) + "<mark>" + esc(text.slice(i, i + q.length)) + "</mark>" + esc(text.slice(i + q.length));
}
// Full path, always LTR, each folder name isolated (<bdi>) so Arabic names never swap places.
// Long paths are shortened in the middle: drive + … + the last folders.
function pathHTML(full, max = 70) {
  let segs = full.split("\\").filter(Boolean);
  if (full.length > max && segs.length > 3) {
    let keep = [segs[segs.length - 1]], len = keep[0].length + segs[0].length + 4;
    for (let i = segs.length - 2; i > 0 && len + segs[i].length + 1 < max; i--) { keep.unshift(segs[i]); len += segs[i].length + 1; }
    segs = [segs[0], "\u2026", ...keep];
  }
  return '<span class="path" dir="ltr">' + segs.map((s) => (s === "\u2026" ? '<span class="ell">…</span>' : `<bdi>${esc(s)}</bdi>`)).join('<span class="sep">\\</span><wbr>') + "</span>";
}

// ---------- strings ----------
const T = {
  ar: {
    app: "InstaFind", appSub: "بحث سريع", settingsTitle: "الإعدادات",
    ph: "ابحث باسم الملف أو داخل المستندات…",
    type: "النوع", typeDocs: "النوع: مستندات", date: "التاريخ", size: "الحجم", folder: "المجلد: كل الأقراص", inside: "داخل المستندات",
    sort: "الأنسب أولاً",
    cName: "الاسم والمسار", cSize: "الحجم", cDate: "آخر تعديل",
    inText: "في النص",
    count: (n, ms) => `عدد النتائج: ${n} (خلال ${ms} ملّي ثانية)`,
    nres: (n) => (n === 0 ? "لا نتائج" : n <= 10 ? `${n} نتائج` : `${n.toLocaleString("en-US")} نتيجة`),
    ixMore: "قد تظهر نتائج أخرى حين تنتهي الفهرسة. لا داعي لإعادة البحث: القائمة تتحدث وحدها.",
    idx: "الفهرس محدَّث · 1,204,331 ملف",
    kNav: "تنقل", kOpen: "فتح", kAct: "إجراءات", kFolder: "المجلد",
    open: "فتح", openFolder: "فتح المجلد", copyPath: "نسخ المسار",
    fType: "النوع", fSize: "الحجم", fMod: "آخر تعديل", fMade: "أُنشئ",
    matchIn: "المطابقة داخل الملف · الصفحة 3", allMatches: "كل التطابقات (7)",
    pdfType: "مستند PDF · 18 صفحة",
    // quick
    best: "أفضل نتيجة", files: "ملفات", insideDocs: "داخل المستندات", answer: "إجابة",
    openFull: (q) => `افتح «${q}» في النافذة الكاملة`, kClose: "إغلاق", floatPh: "ابحث…",
    clock: "2:41 م<br>2026/11/06",
    // empty
    recent: "فتحتها مؤخراً", pinned: "المفضلة", tryThese: "جرّب هذه", all: "الكل",
    trust: "كل شيء يبقى على جهازك. لا نجمع أي بيانات استخدام.",
    tips: [["تقرير .pdf", "ملفات PDF فقط"], ["حجم:>10م", "أكبر من 10 ميغابايت"], ["تاريخ:هذا_الأسبوع", "عُدّلت هذا الأسبوع"], ['"عقد إيجار"', "العبارة كما هي"], ["جذر: كتاب", "مكتبة، الكاتب، كتابات"], ["hgjrvdv", "لوحة المفاتيح الخطأ ← «التقرير»"]],
    // no results
    nrQ: "عقد الايجار 2024",
    nrH: (q) => `لا توجد نتائج لـ «${q}»`,
    nrSub: "بحثنا في أسماء 1,204,331 ملف في كل الأقراص، ضمن المستندات فقط.",
    nr: [["bulb", "هل تقصد «عقد الإيجار 2023»؟", "ملف واحد بهذا الاسم في المستندات"], ["doctext", "ابحث داخل المستندات أيضاً", "يقرأ النص داخل PDF وWord وExcel والصور الممسوحة"], ["filter", "أزل فلتر «النوع: مستندات»", "ابحث في كل الأنواع"], ["globe", "ابحث في الويب", "يفتح المتصفح، ولا يرسل شيئاً غير ما كتبته"]],
    tagline: "<b>لا شيء يضيع بصمت</b>: إذا لم نجد شيئاً نقول لك أين بحثنا وماذا تجرّب.",
    // indexing
    ixQ: "فاتورة",
    ixT: "جارٍ الفهرسة لأول مرة… 36,000 من 432,000 ملف",
    ixS: "بدأنا بمجلداتك: سطح المكتب، المستندات، التنزيلات. يمكنك البحث الآن، وقد تكون النتائج ناقصة حتى ننتهي.",
    ixC: "قراءة نص المستندات: 1,240 من 9,800",
    pause: "إيقاف مؤقت", ixStatus: "جارٍ الفهرسة 8% · حوالي 4 دقائق",
    // settings
    nav: [["general", "عام"], ["search", "البحث"], ["bolt", "البحث من أي مكان"], ["doctext", "داخل المستندات"], ["palette", "المظهر"], ["shield", "الخصوصية والتحديثات"], ["wrench", "متقدم"]],
    sH: "عام", sLead: "تُحفظ التغييرات فوراً.",
    gLang: "اللغة والمظهر", lang: "اللغة", langOpts: ["تلقائي", "العربية", "English"], theme: "المظهر", themeOpts: ["تلقائي", "فاتح", "داكن"],
    digits: "الأرقام العربية (٠١٢٣)", digitsD: "في التواريخ والأحجام وعدد النتائج",
    gAny: "البحث من أي مكان", hotkey: "اختصار نافذة البحث الصغيرة", hotkeyD: "يعمل من أي برنامج", change: "تغيير",
    floatS: "خانة البحث فوق الساعة", floatD: "شفافة حتى تقرّب المؤشر، وتختفي مع ملء الشاشة",
    startup: "التشغيل مع ويندوز", startupD: "في الخلفية، لتعمل Ctrl+Alt+F فور تشغيل الجهاز",
    gRes: "النتائج", density: "كثافة القائمة", densOpts: ["مريحة", "مضغوطة"], hideSys: "إخفاء ملفات النظام", hideSysD: "مجلد Windows والمؤقت والذاكرة المخبأة. اكتب «نظام:» لإظهارها مرة واحدة",
    gPriv: "الاتصال بالإنترنت", rates: "أسعار العملات", ratesD: "تحديث كل 12 ساعة من open.er-api.com", updates: "التحقق من التحديثات", updatesD: "مرة يومياً من GitHub",
    version: "الإصدار 1.0.0 · مجاني ومفتوح المصدر",
    // first run
    welcome: "أهلاً بك في InstaFind", tagline2: "لا شيء يضيع بصمت",
    wLead: "بدأنا فهرسة أقراصك في الخلفية، ويمكنك البحث الآن. سؤال واحد فقط قبل أن تبدأ:",
    wQ: "هل تريد خانة بحث صغيرة فوق الساعة؟",
    wYes: "نعم، أظهرها", wYesD: "تكاد لا تُرى حتى تقرّب المؤشر",
    wNo: "لا، يكفيني الاختصار", wNoD: "Ctrl+Alt+F من أي مكان",
    wStart: "التشغيل مع ويندوز (في الخلفية)", go: "ابدأ البحث",
    wPriv: "كل شيء يبقى على جهازك. لا حساب، ولا بيانات استخدام.",
  },
  en: {
    app: "InstaFind", appSub: "", settingsTitle: "Settings",
    ph: "Search file names or inside documents…",
    type: "Type", typeDocs: "Type: Documents", date: "Date", size: "Size", folder: "Folder: All drives", inside: "Inside documents",
    sort: "Best match first",
    cName: "Name and path", cSize: "Size", cDate: "Modified",
    inText: "In text",
    count: (n, ms) => `${n} results in ${ms} ms`,
    nres: (n) => (n === 0 ? "No results" : `${n.toLocaleString("en-US")} results`),
    ixMore: "More results may appear when indexing finishes. No need to search again: the list updates by itself.",
    idx: "Index up to date · 1,204,331 files",
    kNav: "Move", kOpen: "Open", kAct: "Actions", kFolder: "Folder",
    open: "Open", openFolder: "Open folder", copyPath: "Copy path",
    fType: "Type", fSize: "Size", fMod: "Modified", fMade: "Created",
    matchIn: "Match inside the file · page 3", allMatches: "All matches (7)",
    pdfType: "PDF document · 18 pages",
    best: "Best match", files: "Files", insideDocs: "Inside documents", answer: "Answer",
    openFull: (q) => `Open “${q}” in the full window`, kClose: "Close", floatPh: "Search…",
    clock: "2:41 PM<br>11/6/2026",
    recent: "Recently opened", pinned: "Favourites", tryThese: "Try these", all: "All",
    trust: "Everything stays on your PC. No usage data is collected.",
    tips: [["report .pdf", "PDF files only"], ["size:>10mb", "Bigger than 10 MB"], ["dm:thisweek", "Changed this week"], ['"lease agreement"', "Exact phrase"], ["root: كتاب", "Arabic root: مكتبة، الكاتب"], ["احمد", "Also finds «أحمد»"]],
    nrQ: "lease 2024",
    nrH: (q) => `No results for “${q}”`,
    nrSub: "We searched 1,204,331 file names on all drives, documents only.",
    nr: [["bulb", "Did you mean “lease 2023”?", "One file with that name in Documents"], ["doctext", "Also search inside documents", "Reads the text in PDF, Word, Excel and scanned images"], ["filter", "Remove the “Type: Documents” filter", "Search every file type"], ["globe", "Search the web", "Opens your browser with only what you typed"]],
    tagline: "<b>Nothing gets lost silently</b>: when we find nothing, we tell you where we looked and what to try.",
    ixQ: "invoice",
    ixT: "Indexing for the first time… 36,000 of 432,000 files",
    ixS: "We started with your folders: Desktop, Documents, Downloads. You can search now; results may be incomplete until we finish.",
    ixC: "Reading document text: 1,240 of 9,800",
    pause: "Pause", ixStatus: "Indexing 8% · about 4 minutes",
    nav: [["general", "General"], ["search", "Search"], ["bolt", "Search from anywhere"], ["doctext", "Inside documents"], ["palette", "Appearance"], ["shield", "Privacy and updates"], ["wrench", "Advanced"]],
    sH: "General", sLead: "Changes are saved right away.",
    gLang: "Language and appearance", lang: "Language", langOpts: ["Automatic", "العربية", "English"], theme: "Theme", themeOpts: ["Automatic", "Light", "Dark"],
    digits: "Arabic-Indic digits (٠١٢٣)", digitsD: "In dates, sizes and result counts",
    gAny: "Search from anywhere", hotkey: "Quick search shortcut", hotkeyD: "Works from any app", change: "Change",
    floatS: "Search box above the clock", floatD: "Transparent until you hover; hides in full screen",
    startup: "Start with Windows", startupD: "In the background, so Ctrl+Alt+F works right after sign-in",
    gRes: "Results", density: "List density", densOpts: ["Comfortable", "Compact"], hideSys: "Hide system files", hideSysD: "Windows folder, temp and caches. Type “sys:” to show them once",
    gPriv: "Internet access", rates: "Currency rates", ratesD: "Refreshed every 12 hours from open.er-api.com", updates: "Check for updates", updatesD: "Once a day from GitHub",
    version: "Version 1.0.0 · free and open source",
    welcome: "Welcome to InstaFind", tagline2: "Nothing gets lost silently",
    wLead: "Your drives are being indexed in the background, and you can search right away. Just one question:",
    wQ: "Do you want a small search box above the clock?",
    wYes: "Yes, show it", wYesD: "Almost invisible until you hover",
    wNo: "No, the shortcut is enough", wNoD: "Ctrl+Alt+F from anywhere",
    wStart: "Start with Windows (in the background)", go: "Start searching",
    wPriv: "Everything stays on your PC. No account, no usage data.",
  },
};

// ---------- synthetic data ----------
const U = "C:\\Users\\Sara";
const DATA = {
  ar: {
    q: "تقرير",
    items: [
      { n: "تقرير المبيعات 2026.xlsx", d: U + "\\Documents\\العمل\\المبيعات\\الربع الرابع", t: "xls", s: "48 كيلوبايت", m: "قبل 7 دقائق" },
      { n: "التقرير السنوي النهائي.pdf", d: U + "\\Documents\\العمل\\التقارير\\2026", t: "pdf", s: "2.4 ميغابايت", m: "أمس 4:12 م", pv: true },
      { n: "تقرير الاجتماع الأسبوعي.docx", d: U + "\\Desktop", t: "doc", s: "31 كيلوبايت", m: "اليوم 9:12 ص" },
      { n: "خطة الميزانية.docx", d: U + "\\Documents\\العمل\\المالية", t: "doc", s: "64 كيلوبايت", m: "4 نوفمبر", content: true },
      { n: "صور التقرير الميداني.zip", d: "D:\\Projects\\Field-2026\\Basra", t: "zip", s: "184 ميغابايت", m: "3 نوفمبر" },
      { n: "تقرير_الصيانة_Q3.pptx", d: "D:\\Projects\\Maintenance\\عروض", t: "ppt", s: "6.1 ميغابايت", m: "28 أكتوبر" },
      { n: "غلاف التقرير.png", d: U + "\\Pictures\\تصاميم", t: "img", s: "1.2 ميغابايت", m: "26 أكتوبر" },
      { n: "ملاحظات التقرير.txt", d: U + "\\Documents", t: "txt", s: "2 كيلوبايت", m: "21 أكتوبر" },
      { n: "تقرير الحضور - آذار.xlsx", d: U + "\\OneDrive\\الموارد البشرية\\الحضور والانصراف\\2026", t: "xls", s: "92 كيلوبايت", m: "2 أبريل" },
      { n: "تسجيل التقرير الصوتي.m4a", d: U + "\\Music\\تسجيلات", t: "aud", s: "8.7 ميغابايت", m: "12 مارس" },
      { n: "تقارير 2024", d: "D:\\Archive", t: "dir", s: "—", m: "12 يناير", kw: "تقرير" },
    ],
    pvSnippet: "…ويعرض <mark>التقرير</mark> السنوي النهائي نتائج المبيعات في الربع الرابع مقارنةً بالعام الماضي، مع ملحق للفروع…",
    contentSnippet: "يُرفق <mark>التقرير</mark> المالي مع الخطة",
    pvMade: "2 نوفمبر 2026",
    recent: [
      { n: "عقد إيجار المكتب.pdf", d: U + "\\Documents\\عقود", t: "pdf", m: "قبل ساعة" },
      { n: "جدول الرواتب - تشرين الثاني.xlsx", d: U + "\\Documents\\العمل\\المالية", t: "xls", m: "اليوم 10:02 ص" },
      { n: "عرض المشروع الجديد.pptx", d: "D:\\Projects\\Launch", t: "ppt", m: "أمس" },
      { n: "صورة الهوية.jpg", d: U + "\\Pictures\\مستمسكات", t: "img", m: "أمس" },
      { n: "ملاحظات الاجتماع.txt", d: U + "\\Desktop", t: "txt", m: "3 نوفمبر" },
      { n: "دليل المستخدم.docx", d: U + "\\Downloads", t: "doc", m: "1 نوفمبر" },
    ],
    pins: [{ n: "العمل", t: "dir" }, { n: "فواتير 2026", t: "dir" }, { n: "السيرة الذاتية.docx", t: "doc" }],
    ix: [
      { n: "فاتورة الكهرباء - أكتوبر.pdf", d: U + "\\Downloads", t: "pdf", s: "212 كيلوبايت", m: "قبل يومين" },
      { n: "فاتورة شراء الحاسبة.jpg", d: U + "\\Desktop\\مشتريات", t: "img", s: "1.6 ميغابايت", m: "29 أكتوبر" },
      { n: "فواتير 2026.xlsx", d: U + "\\Documents\\المالية", t: "xls", s: "77 كيلوبايت", m: "25 أكتوبر", kw: "فاتورة" },
      { n: "نموذج فاتورة.docx", d: U + "\\Documents\\قوالب", t: "doc", s: "22 كيلوبايت", m: "9 سبتمبر" },
    ],
    quick: {
      best: { n: "تقرير المبيعات 2026.xlsx", d: U + "\\Documents\\العمل\\المبيعات\\الربع الرابع", t: "xls" },
      files: [
        { n: "التقرير السنوي النهائي.pdf", d: U + "\\Documents\\العمل\\التقارير\\2026", t: "pdf" },
        { n: "تقرير الاجتماع الأسبوعي.docx", d: U + "\\Desktop", t: "doc" },
        { n: "تقارير 2024", d: "D:\\Archive", t: "dir" },
      ],
      inside: { n: "خطة الميزانية.docx", snip: "«…يُرفق <mark>التقرير</mark> المالي مع الخطة…»", t: "doc" },
    },
  },
  en: {
    q: "report",
    items: [
      { n: "Sales report 2026.xlsx", d: U + "\\Documents\\Work\\Sales\\Q4", t: "xls", s: "48 KB", m: "7 min ago" },
      { n: "Annual report - final.pdf", d: U + "\\Documents\\العمل\\Reports\\2026", t: "pdf", s: "2.4 MB", m: "Yesterday 4:12 PM", pv: true },
      { n: "Weekly meeting report.docx", d: U + "\\Desktop", t: "doc", s: "31 KB", m: "Today 9:12 AM" },
      { n: "Budget plan.docx", d: U + "\\Documents\\Work\\Finance", t: "doc", s: "64 KB", m: "Nov 4", content: true },
      { n: "Field report photos.zip", d: "D:\\Projects\\Field-2026\\البصرة", t: "zip", s: "184 MB", m: "Nov 3" },
      { n: "Q3 maintenance report.pptx", d: "D:\\Projects\\Maintenance", t: "ppt", s: "6.1 MB", m: "Oct 28" },
      { n: "report cover.png", d: U + "\\Pictures\\Designs", t: "img", s: "1.2 MB", m: "Oct 26" },
      { n: "report notes.txt", d: U + "\\Documents", t: "txt", s: "2 KB", m: "Oct 21" },
      { n: "Attendance report - March.xlsx", d: U + "\\OneDrive\\HR\\Attendance\\2026\\Monthly", t: "xls", s: "92 KB", m: "Apr 2" },
      { n: "report voice memo.m4a", d: U + "\\Music\\Recordings", t: "aud", s: "8.7 MB", m: "Mar 12" },
      { n: "Reports 2024", d: "D:\\Archive", t: "dir", s: "—", m: "Jan 12" },
    ],
    pvSnippet: "…the final annual <mark>report</mark> compares fourth-quarter sales with last year, with an appendix per branch…",
    contentSnippet: "the finance <mark>report</mark> is attached",
    pvMade: "Nov 2, 2026",
    recent: [
      { n: "Office lease.pdf", d: U + "\\Documents\\Contracts", t: "pdf", m: "1 hour ago" },
      { n: "Payroll - November.xlsx", d: U + "\\Documents\\Work\\Finance", t: "xls", m: "Today 10:02 AM" },
      { n: "New project pitch.pptx", d: "D:\\Projects\\Launch", t: "ppt", m: "Yesterday" },
      { n: "ID scan.jpg", d: U + "\\Pictures\\Papers", t: "img", m: "Yesterday" },
      { n: "meeting notes.txt", d: U + "\\Desktop", t: "txt", m: "Nov 3" },
      { n: "User guide.docx", d: U + "\\Downloads", t: "doc", m: "Nov 1" },
    ],
    pins: [{ n: "Work", t: "dir" }, { n: "Invoices 2026", t: "dir" }, { n: "CV.docx", t: "doc" }],
    ix: [
      { n: "Electricity invoice - October.pdf", d: U + "\\Downloads", t: "pdf", s: "212 KB", m: "2 days ago" },
      { n: "invoice laptop purchase.jpg", d: U + "\\Desktop\\Purchases", t: "img", s: "1.6 MB", m: "Oct 29" },
      { n: "Invoices 2026.xlsx", d: U + "\\Documents\\Finance", t: "xls", s: "77 KB", m: "Oct 25", kw: "invoice" },
      { n: "Invoice template.docx", d: U + "\\Documents\\Templates", t: "doc", s: "22 KB", m: "Sep 9" },
    ],
    quick: {
      best: { n: "Sales report 2026.xlsx", d: U + "\\Documents\\Work\\Sales\\Q4", t: "xls" },
      files: [
        { n: "Annual report - final.pdf", d: U + "\\Documents\\العمل\\Reports\\2026", t: "pdf" },
        { n: "Weekly meeting report.docx", d: U + "\\Desktop", t: "doc" },
        { n: "Reports 2024", d: "D:\\Archive", t: "dir" },
      ],
      inside: { n: "Budget plan.docx", snip: "“…the finance <mark>report</mark> is attached…”", t: "doc" },
    },
  },
};

// ---------- building blocks ----------
const t = () => T[S.lang];
const D = () => DATA[S.lang];

function titlebar(title) {
  const tt = t();
  const name = title ? `${esc(title)} <span>— ${tt.app}</span>` : tt.appSub ? `${tt.app} <span>— ${tt.appSub}</span>` : tt.app;
  return `<header class="titlebar">${LOGO}<div class="name">${name}</div><div class="spacer"></div>
    <div class="caps" aria-hidden="true"><i>${ic("min")}</i><i>${ic("max")}</i><i>${ic("close")}</i></div></header>`;
}
function head(q, meta, focus = true) {
  const tt = t();
  return `<div class="head"><label class="field${focus ? " focus" : ""}">${ic("search")}
      <input id="q" role="combobox" aria-expanded="true" aria-controls="list" aria-autocomplete="list" aria-label="${esc(tt.ph)}" placeholder="${esc(tt.ph)}" value="${esc(q)}" dir="auto" autocomplete="off" spellcheck="false">
      ${meta ? `<span class="meta">${meta}</span>` : ""}</label>
    <button class="iconbtn" aria-label="history">${ic("clock")}</button>
    <button class="iconbtn" data-go="settings" aria-label="settings">${ic("sliders")}</button></div>`;
}
function chips(opts = {}) {
  const tt = t();
  const kind = opts.kindOn ? `<button class="chip on" aria-pressed="true">${ic("file")}${tt.typeDocs}${ic("x", "i caret")}</button>`
    : `<button class="chip">${ic("file")}${tt.type}${ic("down", "i caret")}</button>`;
  return `<div class="chips" role="toolbar">${kind}
    <button class="chip">${ic("cal")}${tt.date}${ic("down", "i caret")}</button>
    <button class="chip">${ic("size")}${tt.size}${ic("down", "i caret")}</button>
    <button class="chip">${ic("drive")}${tt.folder}${ic("down", "i caret")}</button>
    <span class="sep"></span>
    <button class="chip toggle${opts.inside ? " on" : ""}" id="inside" aria-pressed="${!!opts.inside}">${ic("doctext")}${tt.inside}<span class="sw"></span></button>
    <span class="end">${ic("down")}${tt.sort}</span></div>`;
}
function statusbar(left, busy) {
  const tt = t();
  return `<footer class="status" role="status" aria-live="polite"><span class="dot${busy ? " busy" : ""}"></span><span class="num">${left}</span><span class="grow"></span>
    <span class="keys"><span><kbd>↑</kbd><kbd>↓</kbd> ${tt.kNav}</span><span><kbd>Enter</kbd> ${tt.kOpen}</span><span><kbd>Tab</kbd> ${tt.kAct}</span><span><kbd>Ctrl+Alt+F</kbd></span></span></footer>`;
}
function rowHTML(it, i, q, cols = 4) {
  const tt = t();
  const badge = it.content ? `<span class="badge">${tt.inText}</span>` : "";
  const second = it.content && S.inside ? `<span dir="auto">«${D().contentSnippet}»</span> · ${pathHTML(it.d, 40)}` : pathHTML(it.d);
  return `<div class="row" role="option" id="r${i}" data-i="${i}" aria-selected="${i === S.sel}">
    <span>${ft(it.t)}</span>
    <span class="nm"><span class="n1"><bdi class="t">${hl(it.n, q)}</bdi>${badge}</span><span class="n2">${second}</span></span>
    ${cols === 4 ? `<span class="sz num">${esc(it.s)}</span>` : ""}<span class="dt num">${esc(it.m)}</span></div>`;
}
function previewHTML(it) {
  const tt = t();
  if (!it) return `<aside class="preview"></aside>`;
  const isPdf = it.pv;
  const thumb = it.t === "img" ? `<div class="thumb" style="background:linear-gradient(135deg,#78CFFF,#123B2A)"></div>`
    : `<div class="thumb"><div class="page"><b></b><i></i><i></i><i class="hl"></i><i></i><i></i><i></i><i></i></div></div>`;
  return `<aside class="preview" aria-label="preview">${thumb}
    <div><div class="pv-name"><bdi>${hl(it.n, S.q)}</bdi></div><div class="pv-path">${pathHTML(it.d + "\\" + it.n, 999)}</div></div>
    <div class="actions"><button class="btn primary">${ic("open")}${tt.open} <kbd>Enter</kbd></button>
      <button class="btn">${ic("folder")}${tt.openFolder}</button><button class="btn icon" aria-label="${tt.copyPath}">${ic("copy")}</button></div>
    <dl class="facts"><dt>${tt.fType}</dt><dd>${isPdf ? tt.pdfType : esc(it.t.toUpperCase())}</dd><dt>${tt.fSize}</dt><dd class="num">${esc(it.s)}</dd>
      <dt>${tt.fMod}</dt><dd class="num">${esc(it.m)}</dd><dt>${tt.fMade}</dt><dd class="num">${D().pvMade}</dd></dl>
    ${isPdf && S.inside ? `<div class="snippet"><div class="h"><span>${tt.matchIn}</span><span>${tt.allMatches} <kbd>F4</kbd></span></div><p dir="auto">${D().pvSnippet}</p></div>` : ""}
  </aside>`;
}
function filtered() {
  const q = (S.q || "").trim().toLowerCase();
  return D().items.filter((it) => (S.inside || !it.content) && (!q || it.n.toLowerCase().includes(q) || (it.kw || "").includes(q) || (it.content && S.inside)));
}

// ---------- screens ----------
const SCREENS = {
  main() {
    const tt = t();
    if (S.q === null) S.q = D().q;
    const items = filtered();
    const sel = items[Math.min(S.sel, items.length - 1)];
    return `<div class="win">${titlebar()}${head(S.q, `<span class="num">${tt.nres(items.length * 28 + 4)}</span>`)}${chips({ inside: S.inside })}
      <div class="body"><section class="results"><div class="cols"><span></span><span class="sorted">${tt.cName}</span><span>${tt.cSize}</span><span>${tt.cDate}</span></div>
        <div class="list" id="list" role="listbox" aria-label="results" aria-activedescendant="r${S.sel}">${items.map((it, i) => rowHTML(it, i, S.q)).join("")}</div></section>
        ${previewHTML(sel)}</div>
      ${statusbar(tt.count((items.length * 28 + 4).toLocaleString("en-US"), 18) + " · " + tt.idx)}</div>`;
  },

  quick() {
    const tt = t(), qd = D().quick, q = D().q;
    const r = (it, i, best) => `<div class="row${best ? " best" : ""}" role="option" aria-selected="${i === 0}"><span>${ft(it.t)}</span>
      <span class="nm"><span class="n1"><bdi class="t">${hl(it.n, q)}</bdi></span><span class="n2">${it.snip ? `<span dir="auto">${it.snip}</span>` : pathHTML(it.d, 58)}</span></span>
      <span class="hint">${best ? `<kbd>Enter</kbd> ${tt.kOpen}` : ""}</span></div>`;
    return `<div class="desk"><div class="wnd"></div>
      <div class="quick" role="dialog" aria-label="InstaFind">
        <div class="grp"><div class="gh">${tt.best}</div>${r(qd.best, 0, true)}</div>
        <div class="grp"><div class="gh"><span>${tt.files}</span><span class="num">28</span></div>${qd.files.map((f, i) => r(f, i + 1)).join("")}</div>
        <div class="grp"><div class="gh">${tt.insideDocs}</div>${r(qd.inside, 9)}</div>
        <div class="more">${ic("open")}<span class="grow">${esc(tt.openFull(q))}</span><kbd>Ctrl+Enter</kbd></div>
        <div class="qin">${ic("search")}<input value="${esc(q)}" dir="auto" aria-label="${esc(tt.ph)}"><span class="keys"><span><kbd>Tab</kbd> ${tt.kAct}</span><span><kbd>Esc</kbd> ${tt.kClose}</span></span></div>
      </div>
      <div class="floatbox">${ic("search")}${tt.floatPh}</div>
      <div class="taskbar"><i></i><i></i><i class="app">${LOGO}</i><i></i><i></i><i></i><div class="clock num">${tt.clock}</div></div></div>`;
  },

  settings() {
    const tt = t();
    const sw = (on) => `<span class="switch" role="switch" tabindex="0" aria-checked="${on}"></span>`;
    const seg = (opts, on) => `<span class="seg" role="radiogroup">${opts.map((o, i) => `<span role="radio" tabindex="0" aria-checked="${i === on}" class="${i === on ? "on" : ""}" data-seg="${i}">${esc(o)}</span>`).join("")}</span>`;
    const row = (b, s, ctl) => `<div class="srow"><div class="tx"><b>${b}</b>${s ? `<span>${s}</span>` : ""}</div>${ctl}</div>`;
    return `<div class="win">${titlebar(tt.settingsTitle)}
      <div class="settings"><nav class="nav">${tt.nav.map(([i, n], k) => `<a href="#" class="${k === 0 ? "on" : ""}">${ic(i === "search" ? "search" : i)}${n}</a>`).join("")}<div class="foot">${tt.version}</div></nav>
      <main class="sbody"><h1>${tt.sH}</h1><p class="lead">${tt.sLead}</p>
        <div class="group"><h3>${tt.gLang}</h3>
          ${row(tt.lang, "", seg(tt.langOpts, 0))}
          ${row(tt.theme, "", `<span id="themeSeg">${seg(tt.themeOpts, S.theme === "dark" ? 2 : 1)}</span>`)}
          ${row(tt.digits, tt.digitsD, sw(false))}</div>
        <div class="group"><h3>${tt.gAny}</h3>
          ${row(tt.hotkey, tt.hotkeyD, `<span class="hotkey"><kbd>Ctrl</kbd><kbd>Alt</kbd><kbd>F</kbd></span><button class="btn">${tt.change}</button>`)}
          ${row(tt.floatS, tt.floatD, sw(true))}
          ${row(tt.startup, tt.startupD, sw(false))}</div>
        <div class="group"><h3>${tt.gPriv}</h3>
          ${row(tt.updates, tt.updatesD, sw(true))}
          ${row(tt.rates, tt.ratesD, sw(true))}</div>
      </main></div></div>`;
  },

  first() {
    const tt = t();
    return `<div class="win">${titlebar()}<div class="first"><section class="welcome" role="dialog" aria-labelledby="wh">
      <div class="top">${LOGO}<div><h1 id="wh">${tt.welcome}</h1><span class="tag">${tt.tagline2}</span></div>
        <button class="btn ghost lang">${ic("globe")}${S.lang === "ar" ? "English" : "العربية"}</button></div>
      <p class="lead">${tt.wLead}</p>
      <div class="bar small" style="margin:-6px 0 18px"><i style="width:8%"></i></div>
      <p class="q">${tt.wQ}</p>
      <div class="opts" role="radiogroup">
        <div class="opt on" role="radio" aria-checked="true" tabindex="0"><div class="mini"><span class="fb"></span><span class="tb"></span></div><b>${tt.wYes}</b><span>${tt.wYesD}</span></div>
        <div class="opt" role="radio" aria-checked="false" tabindex="0"><div class="mini"><span class="kb"><span><kbd>Ctrl</kbd> <kbd>Alt</kbd> <kbd>F</kbd></span></span><span class="tb"></span></div><b>${tt.wNo}</b><span>${tt.wNoD}</span></div>
      </div>
      <label class="check"><span class="box" role="checkbox" aria-checked="false" tabindex="0"></span>${tt.wStart}</label>
      <div class="foot"><span class="priv">${ic("shield")}${tt.wPriv}</span><button class="btn primary big" data-go="empty">${tt.go}</button></div>
    </section></div></div>`;
  },

  empty() {
    const tt = t(), d = D();
    return `<div class="win">${titlebar()}${head("", "<kbd>Ctrl+K</kbd>")}${chips({})}
      <div class="pane"><div class="home">
        <section><div class="sec-h"><span>${tt.recent}</span><span>${tt.all}</span></div>
          <div role="listbox" aria-label="${tt.recent}">${d.recent.map((it, i) => rowHTML(it, i === 0 ? -1 : i + 100, "", 3)).join("")}</div></section>
        <section><div class="sec-h"><span>${tt.pinned}</span></div>
          <div class="pins">${d.pins.map((p) => `<div class="pin">${ft(p.t)}<span class="t">${esc(p.n)}</span></div>`).join("")}</div>
          <div class="sec-h"><span>${tt.tryThese}</span></div>
          <div class="tips">${tt.tips.map(([c, s]) => `<div class="tip"><code>${esc(c)}</code><span>${esc(s)}</span></div>`).join("")}</div>
          <div class="trust">${ic("shield")}${tt.trust}</div></section>
      </div></div>${statusbar(tt.idx)}</div>`;
  },

  nores() {
    const tt = t(), q = tt.nrQ;
    return `<div class="win">${titlebar()}${head(q, `<span class="num">${tt.nres(0)}</span>`)}${chips({ kindOn: true })}
      <div class="pane"><div class="nores"><div class="card">
        <div class="blob">${ic("search")}</div>
        <h2>${esc(tt.nrH(q))}</h2><p class="sub">${tt.nrSub}</p>
        <div class="sugg" role="listbox">${tt.nr.map(([i, b, s], k) => `<div class="sg${k === 0 ? " sel" : ""}" role="option" aria-selected="${k === 0}"><span class="ic">${ic(i)}</span><span class="tx"><b>${esc(b)}</b><span>${esc(s)}</span></span>${k === 0 ? "<kbd>Enter</kbd>" : k === 1 ? "<kbd>Ctrl+Enter</kbd>" : ""}</div>`).join("")}</div>
        <p class="tagline">${tt.tagline}</p>
      </div></div></div>${statusbar(tt.count(0, 21) + " · " + tt.idx)}</div>`;
  },

  indexing() {
    const tt = t(), d = D();
    const q = tt.ixQ;
    S.q = q; S.sel = 0;
    return `<div class="win">${titlebar()}${head(q, `<span class="num">${tt.nres(4)}</span>`)}${chips({})}
      <div class="body"><section class="results">
        <div class="banner" role="status"><span class="t num">${tt.ixT}</span>
          <div class="btns"><button class="btn">${ic("pause")}${tt.pause}</button></div>
          <span class="s">${tt.ixS}</span>
          <div class="bar"><i style="width:8%"></i></div>
          <span class="s num" style="margin-top:4px">${tt.ixC}</span><div class="bar small lime"><i style="width:13%"></i></div></div>
        <div class="list" id="list" role="listbox" aria-label="results">${d.ix.map((it, i) => rowHTML(it, i, q)).join("")}<p class="inline-note">${ic("refresh")}${tt.ixMore}</p></div></section>
        ${previewHTML(Object.assign({}, d.ix[0], { pv: false }))}</div>
      ${statusbar(tt.ixStatus, true)}</div>`;
  },
};

// ---------- render + interactions ----------
function render() {
  const root = document.documentElement;
  root.lang = S.lang; root.dir = S.lang === "ar" ? "rtl" : "ltr"; root.dataset.theme = S.theme;
  document.title = `InstaFind mockup · ${S.screen} · ${S.lang} · ${S.theme}`;
  document.body.className = S.shot ? "shot" : "";
  document.getElementById("app").innerHTML = (SCREENS[S.screen] || SCREENS.main)() + mockbar();
  const q = document.getElementById("q");
  if (q && !S.shot && S.screen === "main") { q.focus(); q.setSelectionRange(q.value.length, q.value.length); }
}
function mockbar() {
  const b = (k, v, label) => `<button data-${k}="${v}" class="${S[k] === v ? "on" : ""}">${label}</button>`;
  return `<div class="mockbar">${["main", "quick", "settings", "first", "empty", "nores", "indexing"].map((s) => b("screen", s, s)).join("")}
    <span class="sep"></span>${b("lang", "ar", "عربي")}${b("lang", "en", "EN")}<span class="sep"></span>${b("theme", "light", "☀")}${b("theme", "dark", "☾")}</div>`;
}
function syncURL() { if (!S.shot) history.replaceState(null, "", `?screen=${S.screen}&lang=${S.lang}&theme=${S.theme}`); }

document.addEventListener("click", (e) => {
  const el = e.target.closest("[data-screen],[data-lang],[data-theme],[data-go],#inside,.row[data-i],.switch,.seg span,.opt,.box");
  if (!el) return;
  if (el.dataset.screen) { S.screen = el.dataset.screen; S.q = null; S.sel = 1; }
  else if (el.dataset.lang) { S.lang = el.dataset.lang; S.q = null; }
  else if (el.dataset.theme && el.tagName === "BUTTON") { S.theme = el.dataset.theme; }
  else if (el.dataset.go) { S.screen = el.dataset.go; S.q = null; }
  else if (el.id === "inside") { S.inside = !S.inside; }
  else if (el.classList.contains("row")) { S.sel = +el.dataset.i; }
  else if (el.classList.contains("switch")) { el.setAttribute("aria-checked", el.getAttribute("aria-checked") !== "true"); return; }
  else if (el.classList.contains("box")) { el.setAttribute("aria-checked", el.getAttribute("aria-checked") !== "true"); el.textContent = el.getAttribute("aria-checked") === "true" ? "✓" : ""; return; }
  else if (el.classList.contains("opt")) { el.parentNode.querySelectorAll(".opt").forEach((o) => { o.classList.toggle("on", o === el); o.setAttribute("aria-checked", o === el); }); return; }
  else if (el.dataset.seg !== undefined) {
    if (el.closest("#themeSeg")) { S.theme = el.dataset.seg === "2" ? "dark" : "light"; }
    else { el.parentNode.querySelectorAll("span").forEach((s) => { s.classList.toggle("on", s === el); s.setAttribute("aria-checked", s === el); }); return; }
  }
  syncURL(); render();
});
document.addEventListener("input", (e) => {
  if (e.target.id !== "q" || S.screen !== "main") return;
  S.q = e.target.value; S.sel = 0;
  const pos = e.target.selectionStart; render();
  const q = document.getElementById("q"); q.focus(); q.setSelectionRange(pos, pos);
});
document.addEventListener("keydown", (e) => {
  if (e.ctrlKey && e.altKey && e.key.toLowerCase() === "f") { S.screen = S.screen === "quick" ? "main" : "quick"; S.q = null; syncURL(); render(); e.preventDefault(); return; }
  if (e.key === "Escape" && S.screen === "quick") { S.screen = "main"; syncURL(); render(); return; }
  if (S.screen !== "main") return;
  const n = filtered().length;
  if (e.key === "ArrowDown") { S.sel = Math.min(n - 1, S.sel + 1); render(); e.preventDefault(); }
  if (e.key === "ArrowUp") { S.sel = Math.max(0, S.sel - 1); render(); e.preventDefault(); }
});
render();
