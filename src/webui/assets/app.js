// InstaFind main window (src/webui). Talks to the Rust side only through
// window.ipc.postMessage (requests) and window.__instafind.recv (answers);
// see src/webui/bridge.rs for the message types. File names, paths and
// document texts are untrusted: they only ever reach the page through
// textContent. The page names rows ("row 7 of search 42"), never paths,
// and the version of the list they were sent in: the program numbers every
// list it sends and every new order of one, and refuses a row of any other
// version (its place may hold another file by then).
"use strict";
(() => {
  const TOKEN = document.querySelector('meta[name="instafind-token"]').content;
  const LANG = document.documentElement.lang === "en" ? "en" : "ar";
  const FIRST_PAGE_GUARD = 300; // px from the bottom that asks for more rows

  // ---------- strings ----------
  const T = {
    ar: {
      ph: "ابحث باسم الملف…", phContent: "ابحث داخل المستندات ثم اضغط Enter…", type: "النوع", size: "الحجم", date: "التاريخ",
      filters: "عوامل التصفية", lookIn: "البحث في", allDrives: "كل الأقراص", chooseFolder: "اختيار مجلد…", clearFilter: "إزالة",
      inside: "داخل المستندات", sortBy: "الترتيب",
      sorts: { best: "الأنسب أولاً", name: "الاسم", path: "المسار", size: "الحجم", date: "تاريخ التعديل" },
      asc: "تصاعدي", desc: "تنازلي",
      cName: "الاسم والمسار", cSize: "الحجم", cDate: "تاريخ التعديل",
      results: "النتائج", preview: "المعاينة",
      recent: "ملفات فتحتها مؤخراً", recentNone: "الملفات التي تفتحها تظهر هنا.", tryThese: "جرّب هذه",
      trust: "ملفاتك وفهرسها يبقيان على جهازك ولا نجمع بيانات استخدام. يتصل البرنامج بالإنترنت فقط ليتحقق من التحديثات (GitHub) ولأسعار العملات؛ والبحث من الهاتف يعمل داخل شبكة بيتك فقط.",
      tips: [["تقرير .pdf", "ملفات PDF فقط"], ["حجم:>10م", "أكبر من 10 ميغابايت"], ["تاريخ:هذا_الأسبوع", "عُدّلت هذا الأسبوع"], ['"عقد إيجار"', "العبارة كما هي"], ["جذر: كتاب", "مكتبة، الكاتب، كتابات"], ["hgjrvdv", "لوحة المفاتيح الخطأ ← «التقرير»"]],
      kNav: "تنقل", kOpen: "فتح", kFolder: "المجلد", kMenu: "المزيد",
      open: "فتح", openFolder: "فتح المجلد", copyPath: "نسخ المسار", more: "إجراءات أخرى",
      fType: "النوع", fSize: "الحجم", fMod: "تاريخ التعديل", fMatches: "التطابقات", folder: "مجلد", file: "ملف", fileOf: (x) => `ملف ${x}`,
      nres: (n) => (n === 0 ? "لا نتائج" : n === 1 ? "نتيجة واحدة" : n === 2 ? "نتيجتان" : n <= 10 ? `${num(n)} نتائج` : `${num(n)} نتيجة`),
      nMatches: (n) => (n === 1 ? "تطابق واحد" : n === 2 ? "تطابقان" : n <= 10 ? `${num(n)} تطابقات` : `${num(n)} تطابقاً`),
      count: (n, ms) => (n ? `وجدت ${T.nres(n)} في ${num(ms)} ملّي ثانية` : T.nres(0)),
      files: (n) => `الفهرس محدَّث · ${num(n)} ملف`,
      noresH: (q) => `لا توجد نتائج لـ «${q}»`,
      noresSub: (n, kind, scope) => `بحثنا في أسماء ${num(n)} ملف ${scope ? `داخل ${scope}` : "في كل الأقراص"}${kind ? `، ضمن «${kind}» فقط` : ""}.`,
      noresSubContent: (scope) => `بحثنا داخل المستندات ${scope ? `في ${scope}` : "في كل الأقراص"}.`,
      noresIx: "الفهرسة ما زالت جارية، وقد يظهر الملف حين تنتهي.",
      stillIx: "ما زلنا نفهرس ملفاتك…", stillIxSub: "ستظهر النتائج هنا تلقائياً.",
      switchedText: "هذا هو الشكل الجديد لبحث سريع.", dismiss: "إخفاء", matchesPre: "كل التطابقات — ",
      tagHead: "لا شيء يضيع بصمت", tagTail: ": إذا لم نجد شيئاً نقول لك أين بحثنا وماذا تجرّب.",
      loading: "جارٍ تحميل الفهرس…", ixSub: "يمكنك البحث الآن. قد تكون النتائج ناقصة إلى أن تنتهي الفهرسة، وتكتمل القائمة تلقائياً.",
      moreRows: "مرّر لعرض المزيد", searching: "جارٍ البحث داخل المستندات…", pressEnter: "اضغط Enter للبحث داخل المستندات.",
      sugg: { query: "يبحث بهذا النص بدلاً مما كتبت", all_kinds: "يبحث في كل الأنواع", all_drives: "يبحث خارج المجلد المختار", contents: "يقرأ نص المستندات أيضاً", web: "يفتح المتصفح، ولا يرسل شيئاً غير ما كتبته" },
      now: "الآن", minAgo: (n) => (n === 1 ? "قبل دقيقة" : n === 2 ? "قبل دقيقتين" : n <= 10 ? `قبل ${num(n)} دقائق` : `قبل ${num(n)} دقيقة`),
      today: "اليوم", yesterday: "أمس",
      units: ["بايت", "كيلوبايت", "ميغابايت", "غيغابايت", "تيرابايت"],
      stop: "إيقاف", note: "تنبيه",
      // right-click menu
      mOpen: "فتح الملف", mReveal: "فتح المجلد الذي يحتويه", mCopyFile: "نسخ الملف", mCopyPath: "نسخ المسار", mCopyAll: "نسخ مسارات كل النتائج",
      mSizes: "أحجام المجلدات هنا…", mReplace: "استبدال نص في الملفات…", exportBtn: "تصدير النتائج", reindexBtn: "تحديث الفهرس",
      mScope: "البحث في هذا المجلد", mRename: "تغيير الاسم", mRecycle: "نقل إلى سلة المحذوفات", mMatches: "عرض كل التطابقات داخل الملف",
      mSwitch: "فتح مجلدها في نافذة الحفظ/الفتح", qsCancel: "إلغاء النقل",
      // dialogs
      renameH: "تغيير الاسم", renameLabel: "الاسم الجديد", renameOk: "تغيير الاسم", cancel: "إلغاء", close: "إغلاق",
      recycleH: "نقل إلى سلة المحذوفات", recycleQ: ["هل تريد نقل «", "» إلى سلة المحذوفات؟"], recycleQDir: ["هل تريد نقل المجلد «", "» وكل ما فيه إلى سلة المحذوفات؟"],
      recycleSub: "يمكنك استعادته من سلة المحذوفات.", recycleOk: "نقل إلى سلة المحذوفات",
      matchesH: (name) => `كل التطابقات — ${name}`, line: (n) => `سطر ${num(n)}`, firstLines: (n) => `(أول ${num(n)} سطر)`,
      matchesNone: "يُعرض هنا كل سطر يطابق البحث.",
      gone: "نُقل إلى سلة المحذوفات",
      // history
      history: "السجل", historyKey: "السجل (Ctrl+H)", savedH: "البحوث المحفوظة", recentH: "عمليات البحث الأخيرة", recentEmpty: "لا توجد عمليات بحث سابقة بعد",
      saveCurrent: "حفظ البحث الحالي", clearRecent: "مسح عمليات البحث الأخيرة", clearSaved: "حذف كل البحوث المحفوظة",
      savedInside: "داخل المستندات", savedScoped: "في مجلد",
      // settings
      settings: "الإعدادات", settingsKey: "الإعدادات (Ctrl+,)", back: "رجوع إلى البحث",
      sLang: "اللغة", sLangNote: "ستظهر الواجهة باللغة المختارة بعد إعادة تشغيل البرنامج.", restart: "إعادة التشغيل الآن",
      sDisplay: "العرض", sDigits: "عرض الأرقام بالعربية (٠١٢٣)",
      app: "تطبيق", appsLabel: "برامج بهذا الاسم",
      grid: "عرض المصغرات (Ctrl+T)",
      compact: "عرض مضغوط", compactSub: "سطر واحد لكل نتيجة، فتظهر نتائج أكثر", showPreview: "إظهار المعاينة", hidePreview: "إخفاء المعاينة",
      sMore: "المزيد", sMoreRow: "خيارات أخرى", sMoreSub: "التحقق من التحديثات، الدعم والملاحظات، أدوات مثل الملفات المكررة وأحجام المجلدات، وإعدادات البحث من أي مكان", moreBtn: "فتح قائمة الخيارات…", sReportRow: "الإبلاغ عن مشكلة", sReportSub: "تعرض ما سيُشارَك قبل أي شيء: لن يُرسل شيء إلا إذا ضغطت أنت، وأسماء الملفات والمسارات تُحذف.", reportBtn: "الإبلاغ عن مشكلة…",
      sSearch: "البحث والفهرسة", sHideSystem: "إخفاء ملفات النظام", sHideSystemSub: "مجلد ويندوز والملفات المؤقتة وملفات الذاكرة المخبأة",
      sContentIndex: "فهرسة محتوى المستندات في الخلفية", sContentIndexSub: "بحث أسرع داخل الملفات، ويشمل النص في الصور والملفات الممسوحة",
      sReadCloud: "قراءة محتوى الملفات الموجودة على السحابة فقط (يُنزّلها من الإنترنت)",
      sAutoIndex: "تحديث الفهرس تلقائياً عند تغيّر الملفات", reindex: "تحديث الفهرس الآن",
      sAnywhere: "البحث من أي مكان", sHotkey: "Ctrl+Alt+F يفتح خانة البحث الصغيرة؛ عند إيقافه يفتح هذه النافذة",
      sFloating: "خانة بحث شفافة فوق الساعة", sTray: "أيقونة بجانب الساعة",
      sStartup: "التشغيل", sAutostart: "التشغيل مع ويندوز (في الخلفية)",
      sAbout: "حول البرنامج", version: (v) => `بحث سريع (InstaFind) — الإصدار ${v}`,
    },
    en: {
      ph: "Search file names…", phContent: "Search inside documents, then press Enter…", type: "Type", size: "Size", date: "Date",
      filters: "Filters", lookIn: "Look in", allDrives: "All drives", chooseFolder: "Choose folder…", clearFilter: "Remove",
      inside: "Inside documents", sortBy: "Sort",
      sorts: { best: "Best match first", name: "Name", path: "Path", size: "Size", date: "Modified" },
      asc: "ascending", desc: "descending",
      cName: "Name and path", cSize: "Size", cDate: "Modified",
      results: "Results", preview: "Preview",
      recent: "Recently opened files", recentNone: "Files you open show up here.", tryThese: "Try these",
      trust: "Your files and their index stay on your PC; no usage data is collected. InstaFind goes online only to check for updates (GitHub) and fetch exchange rates; phone search works only on your home network.",
      tips: [["report .pdf", "PDF files only"], ["size:>10mb", "Bigger than 10 MB"], ["dm:thisweek", "Changed this week"], ['"lease agreement"', "Exact phrase"], ["root: كتاب", "Arabic root: مكتبة، الكاتب"], ["احمد", "Also finds «أحمد»"]],
      kNav: "Move", kOpen: "Open", kFolder: "Folder", kMenu: "More",
      open: "Open", openFolder: "Open folder", copyPath: "Copy path", more: "More actions",
      fType: "Type", fSize: "Size", fMod: "Modified", fMatches: "Matches", folder: "Folder", file: "File", fileOf: (x) => `${x} file`,
      nres: (n) => (n === 0 ? "No results" : n === 1 ? "1 result" : `${num(n)} results`),
      nMatches: (n) => (n === 1 ? "1 match" : `${num(n)} matches`),
      count: (n, ms) => (n ? `Found ${T.nres(n)} in ${num(ms)} ms` : T.nres(0)),
      files: (n) => `Index up to date · ${num(n)} files`,
      noresH: (q) => `No results for “${q}”`,
      noresSub: (n, kind, scope) => `We searched ${num(n)} file names ${scope ? `in ${scope}` : "on all drives"}${kind ? `, “${kind}” only` : ""}.`,
      noresSubContent: (scope) => `We searched inside documents ${scope ? `in ${scope}` : "on all drives"}.`,
      noresIx: "Indexing is still running; the file may appear when it finishes.",
      stillIx: "Still indexing your files…", stillIxSub: "Results will appear here by themselves.",
      switchedText: "This is InstaFind's new look.", dismiss: "Dismiss", matchesPre: "All matches — ",
      tagHead: "Nothing gets lost silently", tagTail: ": when we find nothing, we tell you where we looked and what to try.",
      loading: "Loading the index…", ixSub: "You can search now; results may be incomplete until we finish. The list updates by itself.",
      moreRows: "Scroll for more", searching: "Searching inside documents…", pressEnter: "Press Enter to search inside documents.",
      sugg: { query: "Searches for this instead", all_kinds: "Searches all types", all_drives: "Looks outside the chosen folder", contents: "Reads the documents' text too", web: "Opens your browser with only what you typed" },
      now: "Just now", minAgo: (n) => (n === 1 ? "1 min ago" : `${num(n)} min ago`),
      today: "Today", yesterday: "Yesterday",
      units: ["bytes", "KB", "MB", "GB", "TB"],
      stop: "Stop", note: "Note",
      mOpen: "Open file", mReveal: "Open containing folder", mCopyFile: "Copy file", mCopyPath: "Copy path", mCopyAll: "Copy paths of all results",
      mSizes: "Folder sizes here…", mReplace: "Replace text in files…", exportBtn: "Export results", reindexBtn: "Update index",
      mScope: "Search in this folder", mRename: "Rename", mRecycle: "Move to Recycle Bin", mMatches: "Show all matches in the file",
      mSwitch: "Open its folder in the Save/Open dialog", qsCancel: "Cancel the move",
      renameH: "Rename", renameLabel: "New name", renameOk: "Rename", cancel: "Cancel", close: "Close",
      recycleH: "Move to the Recycle Bin", recycleQ: ["Move “", "” to the Recycle Bin?"], recycleQDir: ["Move the folder “", "” and everything in it to the Recycle Bin?"],
      recycleSub: "You can restore it from the Recycle Bin.", recycleOk: "Move to Recycle Bin",
      matchesH: (name) => `All matches — ${name}`, line: (n) => `Line ${num(n)}`, firstLines: (n) => `(first ${num(n)} lines)`,
      matchesNone: "Every line that matches the search shows here.",
      gone: "Moved to the Recycle Bin",
      history: "History", historyKey: "History (Ctrl+H)", savedH: "Saved searches", recentH: "Recent searches", recentEmpty: "No recent searches yet",
      saveCurrent: "Save the current search", clearRecent: "Clear recent searches", clearSaved: "Delete all saved searches",
      savedInside: "inside documents", savedScoped: "in a folder",
      settings: "Settings", settingsKey: "Settings (Ctrl+,)", back: "Back to search",
      sLang: "Language", sLangNote: "The interface changes language after the program restarts.", restart: "Restart now",
      sDisplay: "Display", sDigits: "Show Arabic-Indic digits (٠١٢٣)",
      app: "App", appsLabel: "Programs by this name",
      grid: "Thumbnail view (Ctrl+T)",
      compact: "Compact rows", compactSub: "One line per result, so more of them fit", showPreview: "Show the preview", hidePreview: "Hide the preview",
      sMore: "More", sMoreRow: "More options", sMoreSub: "Check for updates, support and feedback, tools such as duplicate files and folder sizes, and search-from-anywhere settings", moreBtn: "Open the Options menu…", sReportRow: "Report a problem", sReportSub: "Shows what would be shared first: nothing is sent unless you press a button, and file names and paths are removed.", reportBtn: "Report a problem…",
      sSearch: "Search and indexing", sHideSystem: "Hide system files", sHideSystemSub: "The Windows folder, temporary and cache files",
      sContentIndex: "Index document contents in the background", sContentIndexSub: "Faster search inside files, text in pictures and scans included",
      sReadCloud: "Read contents of cloud-only files (downloads them)",
      sAutoIndex: "Update the index automatically when files change", reindex: "Update the index now",
      sAnywhere: "Search from anywhere", sHotkey: "Ctrl+Alt+F opens the small search box; when off, it opens this window.",
      sFloating: "Search box above the clock", sTray: "Icon in the notification area",
      sStartup: "Startup", sAutostart: "Start with Windows (in the background)",
      sAbout: "About", version: (v) => `InstaFind — version ${v}`,
    },
  }[LANG];
  const SORTS = ["best", "name", "path", "size", "date"];

  // ---------- state ----------
  const S = {
    seq: 0, shown: -1, ver: 0, recentVer: 0, q: "", kind: 0, size: 0, date: 0, content: false, sort: "best", desc: false, scope: "",
    kinds: [], sizes: [], dates: [], digits: false, version: "",
    rows: [], total: 0, ms: 0, done: true, sel: 0, asked: 0, running: false, isContent: false, contentFor: null,
    progress: "", warnings: [], files: 0, state: "ready", stateText: "",
    recent: [], sugg: [], ssel: 0, view: "empty", note: "", notice: "", resultsScope: "",
    matches: null, history: { recent: [], saved: [] }, settings: null, apps: { seq: -1, apps: [] },
    // Ctrl+Alt+G: a Save/Open dialog waits for a result's folder (its hint), "" when none.
    qswitch: "",
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
    const s = u === 0 ? String(v) : v >= 10 ? v.toFixed(0) : v.toFixed(1).replace(/\.0$/, "");
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

  // The date and time in full (the preview pane), as the classic window's column.
  const fmtFull = new Intl.DateTimeFormat(LOCALE, { day: "numeric", month: "long", year: "numeric", hour: "numeric", minute: "2-digit" });
  function fullDate(secs) { return secs ? digits(fmtFull.format(new Date(secs * 1000))) : ""; }

  // ---------- DOM helpers (no HTML from data, ever) ----------
  function h(tag, attrs, ...kids) {
    const el = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v === null || v === undefined || v === false) continue;
      if (k === "class") el.className = v; else el.setAttribute(k, v === true ? "" : v);
    }
    for (const k of kids) if (k !== null && k !== undefined && k !== false) el.append(k);
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
    up: [["path", { d: "m6 15 6-6 6 6" }]],
    doc: [["path", { d: "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" }], ["path", { d: "M14 3v5h5" }]],
    text: [["path", { d: "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" }], ["path", { d: "M14 3v5h5M9 13h6M9 17h4" }]],
    scale: [["path", { d: "M4 20h16M7 20V10M12 20V4M17 20v-7" }]],
    cal: [["rect", { x: "3", y: "5", width: "18", height: "16", rx: "2" }], ["path", { d: "M3 10h18M8 3v4M16 3v4" }]],
    sort: [["path", { d: "M7 4v16M4 17l3 3 3-3M17 20V4M14 7l3-3 3 3" }]],
    dots: [["circle", { cx: "5", cy: "12", r: "1.4" }], ["circle", { cx: "12", cy: "12", r: "1.4" }], ["circle", { cx: "19", cy: "12", r: "1.4" }]],
    star: [["path", { d: "m12 3 2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1-4.4-4.3 6.1-.9z" }]],
    clock: [["circle", { cx: "12", cy: "12", r: "9" }], ["path", { d: "M12 7v5l3 2" }]],
    trash: [["path", { d: "M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3" }]],
    pen: [["path", { d: "M4 20h4L20 8l-4-4L4 16z" }]],
    back: [["path", { d: "M15 6l-6 6 6 6" }]],
    rows: [["path", { d: "M4 6h16M4 10h16M4 14h16M4 18h16" }]],
    grid: [["rect", { x: "4", y: "4", width: "7", height: "7", rx: "1.5" }], ["rect", { x: "13", y: "4", width: "7", height: "7", rx: "1.5" }], ["rect", { x: "4", y: "13", width: "7", height: "7", rx: "1.5" }], ["rect", { x: "13", y: "13", width: "7", height: "7", rx: "1.5" }]],
    export: [["path", { d: "M12 3v12M7 10l5 5 5-5M5 21h14" }]],
    refresh: [["path", { d: "M20 12a8 8 0 1 1-2.3-5.6M20 4v5h-5" }]],
    pane: [["rect", { x: "3", y: "5", width: "18", height: "14", rx: "2" }], ["path", { d: "M14 5v14" }]],
    flag: [["path", { d: "M5 21V4M5 4h11l-2 4 2 4H5" }]],
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

  // Phone numbers, dates, versions and times («0770-123-4567», «2026-11-12»,
  // «12/11/2026», «2.0.1», «10:30»): groups of digits joined by one - / . : or ,
  // (the runs of src/bidi_layout.rs `digit_runs`). After an Arabic letter the
  // bidi algorithm takes their digits for Arabic numbers, which "-" does not
  // join, so a right-to-left line showed "4567-123-0770" and "12-11-2026".
  // Each run goes in a left-to-right isolate that also overrides the
  // direction of its characters (.run in app.css): an isolate alone keeps
  // "٢٠٢٦-١١-١٢" reversed, ٠–٩ being Arabic numbers in any context. A run is
  // only digits and separators, so the override changes nothing else. The
  // text itself is unchanged.
  const DIGIT_RUN = /[0-9\u0660-\u0669\u06F0-\u06F9]+(?:[-/.:,][0-9\u0660-\u0669\u06F0-\u06F9]+)+/g;
  // Appends text in pieces to `el`, the matched parts in <mark>, each digit
  // run in a <bdi class="run" dir="ltr"> (a match may start or end inside one).
  function fill(el, pieces) {
    const runs = [...pieces.map(([t]) => t).join("").matchAll(DIGIT_RUN)].map((m) => [m.index, m.index + m[0].length]);
    let pos = 0, r = 0, box = null;
    for (const [text, hl] of pieces) {
      for (let i = 0; i < text.length;) {
        const at = pos + i;
        if (r < runs.length && at >= runs[r][1]) { r++; box = null; continue; }
        const inRun = r < runs.length && at >= runs[r][0];
        const end = r < runs.length ? Math.min(text.length, (inRun ? runs[r][1] : runs[r][0]) - pos) : text.length;
        if (inRun && !box) el.append(box = h("bdi", { class: "run", dir: "ltr" }));
        const part = text.slice(i, end);
        (inRun ? box : el).append(hl ? h("mark", null, part) : document.createTextNode(part));
        i = end;
      }
      pos += text.length;
    }
    return el;
  }
  // Text in pieces with the matched parts in <mark>; it takes the direction of its first strong letter.
  function marked(pieces, cls = "t") { return fill(h("bdi", { class: cls }), pieces); }
  function plainName(r) { return r.name.map(([t]) => t).join(""); }

  // A file name as its base and its extension: «تقرير المبيعات ٢٠٢٦.xlsx»
  // in a right-to-left line would show as "xlsx.٢٠٢٦ …" if laid out as one
  // piece of text. The base takes the direction of its first letter, and the
  // extension is a left-to-right isolate at its logical end (on the left of
  // an Arabic name, on the right of an English one), never cut off by a
  // long name. The matched parts stay marked on both sides of the split.
  // Folders, and names whose last dot starts no extension, stay one piece.
  const RTL_LETTER = /[\u0590-\u08FF\uFB1D-\uFDFF\uFE70-\uFEFF]/;
  function extAt(name, folder) {
    const dot = name.lastIndexOf(".");
    if (folder || dot <= 0 || !name.slice(0, dot).trim()) return -1;
    return /^[\p{L}\p{N}_~-]{1,16}$/u.test(name.slice(dot + 1)) && !RTL_LETTER.test(name.slice(dot + 1)) ? dot : -1;
  }
  function firstStrongDir(text) {
    const m = text.match(/\p{L}/u);
    return m ? (RTL_LETTER.test(m[0]) ? "rtl" : "ltr") : "ltr";
  }
  // `pieces` split at character `at` into the parts before and after it.
  function splitPieces(pieces, at) {
    const before = [], after = [];
    let pos = 0;
    for (const [text, hl] of pieces) {
      const cut = Math.max(0, Math.min(text.length, at - pos));
      if (cut > 0) before.push([text.slice(0, cut), hl]);
      if (cut < text.length) after.push([text.slice(cut), hl]);
      pos += text.length;
    }
    return [before, after];
  }
  function fileName(pieces, folder, cls = "t") {
    const name = pieces.map(([t]) => t).join("");
    const dot = extAt(name, folder);
    if (dot < 0) return marked(pieces, cls);
    const [base, ext] = splitPieces(pieces, dot);
    return h("span", { class: cls + " fname", dir: firstStrongDir(name.slice(0, dot)) }, fill(h("bdi", { class: "base", dir: "auto" }), base), fill(h("bdi", { class: "ext", dir: "ltr" }), ext));
  }
  const nameEl = (r) => fileName(r.name, r.folder);

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
      el.append(s === null ? h("span", { class: "ell" }, "…") : fill(h("bdi"), [[s, false]]));
    });
    if (segs.length === 1 && full.endsWith("\\")) el.append(h("span", { class: "sep" }, "\\"));
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
      S.kinds = m.kinds || []; S.sizes = m.sizes || []; S.dates = m.dates || [];
      S.kind = m.kind < S.kinds.length ? m.kind : 0;
      S.sort = SORTS.includes(m.sort) ? m.sort : "best"; S.desc = !!m.desc;
      S.digits = !!m.digits; S.version = m.version || "";
      renderChips(); renderStatus();
      if (m.switched) showSwitched();
    },
    results(m) {
      if (m.seq !== S.seq) return; // an older search: the user typed on
      // The same search again (the index changed, a document search ended,
      // another order): keep the selected file.
      const refresh = m.seq === S.shown && S.rows[S.sel];
      const was = refresh && joinPath(S.rows[S.sel].dir, plainName(S.rows[S.sel]));
      const keep = refresh ? m.rows.findIndex((r) => joinPath(r.dir, plainName(r)) === was) : -1;
      S.shown = m.seq;
      S.rows = m.rows; S.total = m.total; S.ms = m.ms; S.done = m.rows.length >= m.total;
      S.asked = 0; S.note = m.note || ""; S.sugg = m.suggestions || []; S.ssel = 0;
      S.files = m.files ?? S.files; S.kindLabel = m.kind_label || ""; S.resultsScope = m.scope || "";
      S.isContent = !!m.content; S.running = !!m.running; S.warnings = m.warnings || [];
      if (!S.running && !S.isContent) S.progress = "";
      // Another version may hold other files at the same places.
      if (!refresh || m.ver !== S.ver) S.matches = null;
      S.ver = m.ver;
      S.sel = keep > 0 ? keep : 0;
      render(!!refresh);
      announce();
    },
    rows(m) {
      if (m.seq !== S.shown || m.ver !== S.ver || m.from !== S.rows.length) return;
      S.rows = S.rows.concat(m.rows); S.done = S.rows.length >= S.total; S.asked = 0;
      if (S.view !== "results") render(); else appendRows(m.from);
    },
    progress(m) {
      if (m.seq !== S.seq) return;
      S.total = Math.max(S.total, m.total); S.running = !m.done; S.progress = m.text || "";
      S.done = S.rows.length >= S.total;
      if (!S.done) wantMore();
      if (S.view === "results" && !S.rows.length) renderRows(false);
      renderStatus(); renderStop();
      if (m.done) announce();
    },
    matches(m) {
      if (m.seq !== S.shown || m.ver !== S.ver) return;
      S.matches = m;
      const r = S.rows[S.sel];
      if (r && r.id === m.id) renderPreview();
      if (dlgFor === "matches:" + m.id) showMatches(m.id);
    },
    recent(m) { S.recent = m.rows || []; S.recentVer = m.ver || 0; renderRecent(); },
    history(m) { S.history = { recent: m.recent || [], saved: m.saved || [] }; if (!$("hist").hidden) renderHistory(); },
    scope(m) {
      S.scope = m.path || "";
      renderChips();
      if (m.search && S.q.trim()) runSearch();
    },
    apply(m) {
      q.value = m.q; S.q = m.q;
      S.kind = m.kind < S.kinds.length ? m.kind : 0; S.content = !!m.content;
      closeSettings(); renderChips(); q.focus();
      runSearch();
    },
    renamed(m) {
      if (m.list !== "results" || m.seq !== S.shown || m.ver !== S.ver) return;
      const r = S.rows[m.id];
      if (!r) return;
      r.name = [[m.name, false]];
      const dot = m.name.lastIndexOf(".");
      if (!r.folder) r.ext = dot > 0 ? m.name.slice(dot + 1).toLowerCase() : "";
      replaceRow(m.id);
    },
    removed(m) {
      if (m.list !== "results" || m.seq !== S.shown || m.ver !== S.ver || !S.rows[m.id]) return;
      S.rows[m.id].gone = true;
      replaceRow(m.id);
    },
    settings(m) {
      const hide = S.settings && S.settings.hide_system !== m.hide_system;
      S.settings = m;
      if (S.digits !== !!m.digits) { S.digits = !!m.digits; render(true); renderRecent(); }
      if (S.view === "settings") renderSettings();
      if (hide && S.q.trim() && !S.content) runSearch();
    },
    status(m) {
      const was = S.state;
      S.state = m.state; S.stateText = m.text || ""; S.files = m.files;
      // Indexing ended with nothing listed: search again (the program does
      // too when its index changes; this covers a list it never sent).
      if (was !== "ready" && m.state === "ready" && S.view === "nores" && !S.content && S.q.trim()) runSearch();
      renderBanner(); renderStatus();
      if (S.view === "nores") renderNoResults();
    },
    // A search from the quick popup; its suggestions say how to run it
    // (inside documents, every file type; "all drives" came as a scope).
    query(m) {
      closeSettings(); q.value = m.q;
      if (m.all_kinds) S.kind = 0;
      if (m.content) S.content = true;
      if (m.all_kinds || m.content) renderChips();
      if (m.content) { S.q = q.value; runSearch(); } else onInput();
    },
    focus() { q.focus(); },
    thumbs(m) {
      if (m.seq !== S.shown || m.ver !== S.ver) return;
      for (const [id, key] of m.keys || []) {
        thumbKeys.set(`${m.seq}:${m.ver}:${id}`, key);
        const tile = $("r" + id);
        if (tile && tile.classList.contains("tile")) showThumb(tile, key);
      }
    },
    apps(m) { S.apps = { seq: m.seq, apps: m.apps || [] }; renderApps(); },
    quick_switch(m) {
      S.qswitch = m.on ? m.text || "" : "";
      renderQswitch();
      if (m.on) { closeSettings(); $("live").textContent = m.text || ""; }
    },
    notice(m) {
      S.notice = m.text; S.noticeWarn = !!m.warn; renderStatus();
      clearTimeout(handlers.noticeTimer);
      // A problem stays until the next key or click, and is read out.
      if (m.warn) $("live").textContent = m.text;
      else handlers.noticeTimer = setTimeout(() => { S.notice = ""; renderStatus(); }, 3500);
    },
  };
  Object.defineProperty(window, "__instafind", { value: Object.freeze({ recv: (m) => { const f = handlers[m && m.type]; if (f) f(m); } }) });

  // ---------- searching ----------
  function runSearch() {
    S.q = q.value;
    S.seq++;
    if (!S.q.trim()) { S.rows = []; S.total = 0; S.sel = 0; S.running = false; render(); return; }
    if (S.content) { S.contentFor = S.q; S.rows = []; S.total = 0; S.running = true; S.progress = T.searching; }
    send({ op: "search", seq: S.seq, q: S.q, kind: S.kind, size: S.size, date: S.date, content: S.content, sort: S.sort, desc: S.desc });
    renderStop();
  }
  function onInput() {
    if (S.view === "settings") closeSettings();
    S.q = q.value;
    if (!S.q.trim()) { S.seq++; S.rows = []; S.total = 0; S.sel = 0; S.running = false; S.contentFor = null; render(); return; }
    // Reading every document on each keystroke would be slow: the search
    // inside documents runs on Enter, as in the classic window.
    if (S.content) { renderStatus(); return; }
    runSearch();
  }
  // The search box's text is not what the shown document results are for.
  const contentStale = () => S.content && S.contentFor !== q.value;
  // The list the rows on screen were sent in: what every request about a
  // row names. Taken when a menu or dialog opens, so it acts on the list it
  // was opened for even if a newer one came meanwhile.
  function listAt(listName = "results") {
    return listName === "results" ? { list: listName, seq: Math.max(S.shown, 0), ver: S.ver } : { list: listName, seq: 0, ver: S.recentVer };
  }
  function askMore() {
    if (S.done || S.asked || S.view !== "results") return;
    S.asked = S.rows.length || -1;
    send({ op: "more", seq: Math.max(S.shown, 0), ver: S.ver, from: S.rows.length });
  }
  // More rows are there (a document search found them): fetch them if the list is short or near its end.
  function wantMore() {
    if (S.asked) return;
    if (S.rows.length < 60 || list.scrollTop + list.clientHeight > list.scrollHeight - FIRST_PAGE_GUARD) {
      S.asked = S.rows.length || -1;
      send({ op: "more", seq: Math.max(S.shown, 0), ver: S.ver, from: S.rows.length });
    }
  }
  function act(op, row, listName = "results", at = listAt(listName)) {
    if (!row || row.gone) return;
    send({ op, list: at.list, seq: at.seq, ver: at.ver, id: row.id });
  }
  function stop() { if (S.running) send({ op: "stop", seq: S.seq }); }

  // ---------- rendering ----------
  function setView(v) {
    S.view = v;
    for (const id of ["results", "empty", "nores", "settings"]) $("view-" + id).hidden = v !== id;
    $("chips").hidden = v === "settings";
    $("set-btn").setAttribute("aria-pressed", String(v === "settings"));
    q.setAttribute("aria-expanded", String(v === "results"));
    q.setAttribute("aria-controls", v === "nores" ? "sugg" : "list");
  }
  function render(refresh = false) {
    if (S.view === "settings") { renderStatus(); renderBanner(); return; }
    if (!S.q.trim()) setView("empty");
    else setView(S.rows.length || S.running ? "results" : "nores");
    // (Not "no results" while the first indexing may still find them.)
    const waiting = !S.total && !S.isContent && (S.state === "names" || S.state === "loading");
    $("meta").textContent = S.q.trim() && !S.running && !waiting ? T.nres(S.total) : "";
    if (S.view === "results") { renderCols(); renderRows(refresh); }
    if (S.view === "nores") renderNoResults();
    renderPreview();
    renderStatus(); renderStop();
    renderBanner(); renderApps();
  }
  // Programs the search names (as the classic window's app buttons): a
  // click starts one; the page names it by its place in the list sent.
  // Once, for someone the update moved from the classic window: what
  // happened (the new look is the only one since 1.0).
  function showSwitched() {
    const bar = $("switched");
    const x = btn("icon", I.x, null, () => { bar.hidden = true; q.focus(); }, { "aria-label": T.dismiss, title: T.dismiss, id: "switched-x" });
    bar.replaceChildren(h("span", null, T.switchedText), x);
    bar.hidden = false;
  }
  // Ctrl+Alt+G in a Save/Open dialog: what Enter and Esc do now, until the
  // dialog was moved or the move cancelled (the program says when).
  function renderQswitch() {
    const bar = $("qswitch");
    bar.hidden = !S.qswitch;
    if (!S.qswitch) { bar.replaceChildren(); return; }
    const x = btn("small", I.x, T.qsCancel, () => { send({ op: "switch_cancel" }); q.focus(); }, { id: "qswitch-x" });
    // "Enter: … · Esc: …": each key as a key cap (a bare "Enter:" in an
    // Arabic line would show its colon on the wrong side).
    const say = h("span", null);
    S.qswitch.split(" · ").forEach((part, i) => {
      const m = part.match(/^([A-Za-z][A-Za-z0-9+]*):\s*(.*)$/);
      if (i) say.append(" · ");
      if (m) say.append(h("kbd", null, m[1]), " ", m[2]); else say.append(part);
    });
    bar.replaceChildren(say, x);
  }
  // The selected result (the top one from the box) for the waiting dialog.
  function switchTo(row, listName = "results", at = listAt(listName)) {
    if (!row || row.gone) return;
    send({ op: "switch_to", list: at.list, seq: at.seq, ver: at.ver, id: row.id });
  }
  // ---------- how the results are laid out (kept in this page's storage) ----------
  // Compact rows (one line each) unless chosen otherwise: on by itself in a
  // window under 800 px high. The preview pane can be hidden.
  const PREF = (() => { try { return JSON.parse(localStorage.getItem("instafind.view") || "{}") || {}; } catch { return {}; } })();
  function savePref() { try { localStorage.setItem("instafind.view", JSON.stringify(PREF)); } catch { /* not kept */ } }
  function compactOn() { return typeof PREF.compact === "boolean" ? PREF.compact : innerHeight < 800; }
  function previewOn() { return PREF.preview !== false; }
  function gridOn() { return PREF.grid === true; }
  function applyView() {
    document.body.classList.toggle("compact", compactOn());
    document.body.classList.toggle("gridview", gridOn());
    list.classList.toggle("grid", gridOn());
    $("view-results").classList.toggle("nopreview", !previewOn());
  }
  const ZOOMS = [0.8, 0.9, 1, 1.1, 1.25, 1.4, 1.6];
  function applyZoom() { document.documentElement.style.zoom = String(ZOOMS.includes(PREF.zoom) ? PREF.zoom : 1); }
  function zoomBy(step) {
    const i = ZOOMS.indexOf(ZOOMS.includes(PREF.zoom) ? PREF.zoom : 1);
    PREF.zoom = step === 0 ? 1 : ZOOMS[Math.max(0, Math.min(ZOOMS.length - 1, i + step))];
    savePref(); applyZoom();
  }
  document.addEventListener("wheel", (e) => { if (e.ctrlKey) { zoomBy(e.deltaY < 0 ? 1 : -1); e.preventDefault(); } }, { passive: false });
  const COLW = { size: 96, date: 128 };
  function colWidth(key) { const w = PREF.cols && PREF.cols[key]; return typeof w === "number" ? w : COLW[key]; }
  function setColWidth(key, w, save = true) {
    PREF.cols = Object.assign({}, PREF.cols, { [key]: Math.round(Math.max(64, Math.min(320, w))) });
    applyCols();
    if (save) savePref();
  }
  function applyCols() {
    document.body.style.setProperty("--w-size", colWidth("size") + "px");
    document.body.style.setProperty("--w-date", colWidth("date") + "px");
  }
  function setPref(key, on) { PREF[key] = on; savePref(); applyView(); renderChips(); if (S.view === "settings") renderSettings(); }
  function viewButtons() {
    const b = (id, ic, on, label, click) => {
      const el = btn("icon view", ic, null, click, { id, "aria-pressed": String(on), "aria-label": label, title: label });
      return el;
    };
    const tools = [
      btn("small", I.export, T.exportBtn, () => send({ op: "export" }), { id: "tool-export" }),
      btn("small", I.refresh, T.reindexBtn, () => send({ op: "reindex" }), { id: "tool-reindex" }),
    ];
    return h("span", { class: "viewbtns" }, ...tools,
      b("view-grid", I.grid, gridOn(), T.grid, () => toggleGrid()),
      b("view-compact", I.rows, compactOn(), T.compact, () => setPref("compact", !compactOn())),
      b("view-preview", I.pane, previewOn(), previewOn() ? T.hidePreview : T.showPreview, () => setPref("preview", !previewOn())));
  }
  window.addEventListener("resize", () => { if (typeof PREF.compact !== "boolean") { applyView(); renderChips(); } });
  function renderApps() {
    const box = $("apps");
    const show = S.apps.seq === S.seq && S.apps.apps.length > 0 && S.q.trim() && !S.content && (S.view === "results" || S.view === "nores");
    box.hidden = !show;
    if (!show) { box.replaceChildren(); return; }
    box.replaceChildren(...S.apps.apps.map((title, id) => btn("app", I.open, `${T.app}: ${title}`, () => send({ op: "app", seq: S.apps.seq, id }), { "data-app": String(id) })));
  }
  // ---------- the thumbnail view (Ctrl+T) ----------
  // Results as tiles: the file-type badge until the program's thumbnail
  // arrives. Only the tiles that come into view ask for one, by row (the
  // program answers with a one-time key per row; the picture comes from
  // its own protocol, /thumb/<key>; the page never names a path).
  const thumbKeys = new Map(); // "seq:ver:id" -> key, or "" while asked
  let thumbWant = [], thumbTimer = 0;
  const thumbSeen = new IntersectionObserver((entries) => {
    for (const en of entries) {
      if (!en.isIntersecting) continue;
      thumbSeen.unobserve(en.target);
      const id = +en.target.dataset.i, k = `${S.shown}:${S.ver}:${id}`;
      if (!thumbKeys.has(k)) { thumbKeys.set(k, ""); thumbWant.push(id); }
    }
    if (thumbWant.length && !thumbTimer) thumbTimer = setTimeout(askThumbs, 60);
  }, { root: list, rootMargin: "240px 0px" });
  function askThumbs() {
    thumbTimer = 0;
    if (thumbKeys.size > 4000) thumbKeys.clear();
    while (thumbWant.length) send({ op: "thumbs", seq: S.shown, ver: S.ver, ids: thumbWant.splice(0, 64) });
  }
  function showThumb(tile, key) {
    const img = tile.querySelector("img");
    if (!img || !key) return;
    img.addEventListener("load", () => tile.classList.add("pic"), { once: true });
    img.src = "/thumb/" + key;
  }
  function tileEl(r, i) {
    const name = plainName(r);
    const el = h("div", { class: "row tile" + (r.gone ? " gone" : ""), role: "option", id: "r" + i, "aria-selected": String(i === S.sel), "aria-label": `${name}, ${r.dir}${r.folder ? "" : ", " + size(r.size)}, ${when(r.mtime)}${r.gone ? ", " + T.gone : ""}` });
    el.dataset.i = i;
    el.append(h("span", { class: "tpic" }, badge(r, "ft big"), h("img", { alt: "", draggable: "false" })), h("span", { class: "tname" }, nameEl(r)));
    const key = thumbKeys.get(`${S.shown}:${S.ver}:${i}`);
    if (key) showThumb(el, key);
    else if (!r.gone) thumbSeen.observe(el);
    return el;
  }
  function toggleGrid() {
    PREF.grid = !gridOn(); savePref(); applyView(); renderChips();
    if (S.view === "results") { renderCols(); renderRows(true); }
  }

  function rowEl(r, i, listName) {
    if (listName === "results" && gridOn()) return tileEl(r, i);
    const name = plainName(r);
    const el = h("div", { class: "row" + (r.gone ? " gone" : "") + (r.snippet ? " doc" : ""), role: "option", id: (listName === "recent" ? "rr" : "r") + i, "aria-selected": String(listName === "results" && i === S.sel), "aria-label": `${name}, ${r.dir}${r.folder ? "" : ", " + size(r.size)}, ${when(r.mtime)}${r.gone ? ", " + T.gone : ""}` });
    el.dataset.i = i;
    const nm = h("span", { class: "nm" }, h("span", { class: "n1" }, nameEl(r), r.matches > 1 ? h("span", { class: "cnt num" }, T.nMatches(r.matches)) : null), h("span", { class: "n2" }, pathEl(r.dir)));
    if (r.snippet) nm.append(h("span", { class: "n3" }, marked(r.snippet, "sn")));
    el.append(h("span", null, badge(r)), nm);
    if (listName === "results") el.append(h("span", { class: "sz num" }, r.folder ? "" : size(r.size)));
    el.append(h("span", { class: "dt num" }, when(r.mtime)));
    return el;
  }
  function replaceRow(i) {
    const old = $("r" + i);
    if (old) { old.replaceWith(rowEl(S.rows[i], i, "results")); if (i === S.sel) select(i, false); }
    if (i === S.sel) renderPreview();
  }
  function renderCols() {
    const col = (key, label, cls) => {
      const on = S.sort === key;
      const b = h("button", { class: "colbtn" + (on ? " sorted" : "") + (cls ? " " + cls : ""), type: "button", "aria-pressed": String(on) }, h("span", null, label));
      if (on) b.append(icon(S.desc ? I.down : I.up, "i caret"));
      b.dataset.sort = key;
      return b;
    };
    // The Size and Date columns can be made wider or narrower by dragging
    // the edge before their header (kept for next time).
    const grip = (key) => {
      const g = h("span", { class: "grip", role: "separator", "aria-orientation": "vertical", "aria-label": key === "size" ? T.cSize : T.cDate });
      g.addEventListener("pointerdown", (e) => {
        e.preventDefault(); e.stopPropagation();
        const start = e.clientX, w0 = colWidth(key), rtl = document.documentElement.dir === "rtl";
        g.setPointerCapture(e.pointerId); g.classList.add("on");
        const move = (ev) => setColWidth(key, w0 + (rtl ? ev.clientX - start : start - ev.clientX), false);
        const up = () => { g.classList.remove("on"); g.removeEventListener("pointermove", move); savePref(); };
        g.addEventListener("pointermove", move);
        g.addEventListener("pointerup", up, { once: true });
      });
      g.addEventListener("click", (e) => e.stopPropagation());
      return g;
    };
    const size = col("size", T.cSize, "end"), date = col("date", T.cDate, "end");
    $("cols").replaceChildren(h("span"), col("name", T.cName), h("span", { class: "cwrap end" }, grip("size"), size), h("span", { class: "cwrap end" }, grip("date"), date));
  }
  function renderRows(refresh) {
    const top = list.scrollTop;
    thumbSeen.disconnect();
    list.replaceChildren(...S.rows.map((r, i) => rowEl(r, i, "results")));
    if (!S.done || (S.running && !S.rows.length)) list.append(h("div", { class: "more", "aria-hidden": "true" }, S.running && !S.rows.length ? T.searching : T.moreRows));
    list.scrollTop = refresh ? top : 0;
    select(S.sel, refresh);
  }
  function appendRows(from) {
    const more = list.querySelector(".more");
    const frag = document.createDocumentFragment();
    S.rows.slice(from).forEach((r, k) => frag.append(rowEl(r, from + k, "results")));
    if (more) { more.before(frag); more.textContent = T.moreRows; } else list.append(frag);
    if (S.done && more) more.remove();
    if (from === 0) select(0, false);
  }
  let matchTimer = 0;
  function select(i, scroll = true) {
    if (!S.rows.length) { q.removeAttribute("aria-activedescendant"); list.removeAttribute("aria-activedescendant"); renderPreview(); return; }
    S.sel = Math.max(0, Math.min(S.rows.length - 1, i));
    for (const el of list.querySelectorAll('.row[aria-selected="true"]')) el.setAttribute("aria-selected", "false");
    const el = $("r" + S.sel);
    if (el) { el.setAttribute("aria-selected", "true"); if (scroll) el.scrollIntoView({ block: "nearest" }); }
    q.setAttribute("aria-activedescendant", "r" + S.sel);
    list.setAttribute("aria-activedescendant", "r" + S.sel);
    if (S.sel > S.rows.length - 20) askMore();
    // Every matching line of a document, once the selection rests on it.
    const r = S.rows[S.sel];
    clearTimeout(matchTimer);
    if (r.snippet && !(S.matches && S.matches.id === r.id)) { const at = listAt(); matchTimer = setTimeout(() => send({ op: "matches", seq: at.seq, ver: at.ver, id: r.id }), 120); }
    renderPreview();
  }
  function btn(cls, ic, label, onClick, extra) {
    const b = h("button", Object.assign({ class: "btn " + cls, type: "button" }, extra), icon(ic));
    if (label) b.append(label);
    b.addEventListener("click", onClick);
    return b;
  }
  function linesEl(m, max) {
    const box = h("ol", { class: "lines" });
    for (const [n, pieces] of m.lines.slice(0, max)) box.append(h("li", null, h("span", { class: "ln num" }, T.line(n)), marked(pieces, "lt")));
    return box;
  }
  // The selected result's actions under the list, when the preview pane is
  // not there (a narrow window, or hidden).
  function renderActbar(r) {
    const bar = $("actbar");
    if (!r || r.gone) { bar.replaceChildren(); return; }
    bar.replaceChildren(
      btn("primary small", I.open, T.open, () => act("open", S.rows[S.sel])),
      btn("small", I.folder, T.openFolder, () => act("reveal", S.rows[S.sel])),
      btn("small", I.copy, T.copyPath, () => act("copy", S.rows[S.sel])),
      btn("icon small", I.dots, null, (e) => openCtx(S.sel, e.currentTarget), { "aria-label": T.more, title: T.more, "aria-haspopup": "menu" }));
  }
  function renderPreview() {
    const pv = $("preview");
    const r = S.view === "results" ? S.rows[S.sel] : null;
    renderActbar(r);
    if (!r) { pv.replaceChildren(); return; }
    // A document found by its text: a small icon, so its matching lines show.
    pv.classList.toggle("doc", !!r.snippet);
    const kind = r.folder ? T.folder : r.ext ? T.fileOf(r.ext.toUpperCase()) : T.file;
    const facts = h("dl", { class: "facts" }, h("dt", null, T.fType), h("dd", null, kind));
    if (!r.folder) facts.append(h("dt", null, T.fSize), h("dd", { class: "num" }, size(r.size)));
    facts.append(h("dt", null, T.fMod), h("dd", { class: "num", title: fullDate(r.mtime) }, fullDate(r.mtime)));
    if (r.matches) facts.append(h("dt", null, T.fMatches), h("dd", { class: "num" }, T.nMatches(r.matches)));
    const open = btn("primary", I.open, T.open + " ", () => act("open", S.rows[S.sel]), { disabled: r.gone });
    open.append(h("kbd", null, "Enter"));
    const parts = [
      h("div", { class: "thumb" }, badge(r)),
      h("div", null, h("div", { class: "pv-name" }, nameEl(r)), h("div", { class: "pv-path" }, pathEl(r.dir, 999))),
    ];
    if (r.gone) parts.push(h("p", { class: "gone-note" }, T.gone));
    else parts.push(h("div", { class: "actions" }, open, btn("", I.folder, T.openFolder, () => act("reveal", S.rows[S.sel])),
      btn("", I.copy, T.copyPath, () => act("copy", S.rows[S.sel])),
      btn("icon", I.dots, null, (e) => openCtx(S.sel, e.currentTarget), { "aria-label": T.more, title: T.more, "aria-haspopup": "menu" })));
    parts.push(facts);
    if (r.snippet) {
      const m = S.matches && S.matches.id === r.id ? S.matches : null;
      const sec = h("section", { class: "pv-lines" }, h("div", { class: "sec-h" }, h("span", null, T.mMatches), h("kbd", null, "F4")));
      sec.append(m ? linesEl(m, 12) : h("p", { class: "snip" }, marked(r.snippet, "sn")));
      parts.push(sec);
    }
    pv.replaceChildren(...parts);
  }
  function renderRecent() {
    const el = $("recent");
    el.replaceChildren(...S.recent.map((r, i) => rowEl(r, i, "recent")));
    $("recent-none").hidden = S.recent.length > 0;
  }
  const SUGG_ICON = { query: I.bulb, all_kinds: I.filter, all_drives: I.folder, contents: I.text, web: I.globe };
  function renderNoResults() {
    // The first indexing is still under way: say so, not "nothing in 0 files".
    // The program lists the results by itself when the index is ready.
    const indexing = !S.isContent && (S.state === "names" || S.state === "loading");
    $("view-nores").classList.toggle("indexing", indexing);
    if (indexing) {
      $("nores-h").textContent = T.stillIx;
      $("nores-sub").textContent = T.stillIxSub;
      $("sugg").replaceChildren();
      q.removeAttribute("aria-activedescendant");
      return;
    }
    $("nores-h").textContent = T.noresH(S.q.trim());
    const scope = S.resultsScope ? S.resultsScope : "";
    let sub = S.isContent ? T.noresSubContent(scope) : T.noresSub(S.files, S.kind ? S.kindLabel : "", scope);
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
    if (s.kind === "query") { q.value = s.q; runSearch(); }
    else if (s.kind === "all_kinds") setFilter("kind", 0);
    else if (s.kind === "all_drives") send({ op: "all_drives" });
    else if (s.kind === "contents") { S.content = true; renderChips(); runSearch(); }
    else if (s.kind === "web") send({ op: "web", q: S.q });
  }
  // The indexing banner: in full for its first seconds, then one line.
  let bannerSince = 0;
  function renderBanner() {
    const busy = S.state !== "ready";
    $("banner").hidden = !busy || S.view === "settings";
    if (!busy) { bannerSince = 0; $("banner").classList.remove("slim"); return; }
    if (!bannerSince) {
      bannerSince = Date.now();
      setTimeout(() => { if (bannerSince) $("banner").classList.add("slim"); }, 6000);
    }
    $("banner-title").textContent = S.state === "loading" ? T.loading : digits(S.stateText);
    $("banner-sub").textContent = T.ixSub;
    $("banner-bar").classList.add("busy");
  }
  function renderStatus() {
    const busy = S.state !== "ready";
    $("dot").classList.toggle("busy", busy || S.running);
    let text;
    if (S.notice) text = S.notice;
    else if (busy) text = S.state === "loading" ? T.loading : digits(S.stateText);
    else text = T.files(S.files);
    if (!S.notice && S.view !== "empty" && S.view !== "settings") {
      if (S.content && contentStale()) text = T.pressEnter + " · " + text;
      else if (S.isContent && S.progress) text = digits(S.progress);
      else text = (S.note ? S.note + " · " : "") + T.count(S.total, S.ms) + " · " + text;
      if (S.warnings.length) text += ` — ${T.note}: ${S.warnings.join(LANG === "ar" ? "، " : "; ")}`;
    } else if (!S.notice && S.content && contentStale() && S.q.trim()) text = T.pressEnter;
    $("status-text").replaceChildren();
    fill($("status-text"), [[text, false]]);
    $("status-text").classList.toggle("warn", !!(S.notice && S.noticeWarn));
  }
  for (const ev of ["keydown", "mousedown"]) document.addEventListener(ev, () => { if (S.notice && S.noticeWarn) { S.notice = ""; S.noticeWarn = false; renderStatus(); } }, true);
  function renderStop() { $("stop").hidden = !S.running; }
  // The result count for screen readers, once typing pauses.
  let liveTimer = 0;
  function announce() {
    clearTimeout(liveTimer);
    liveTimer = setTimeout(() => { $("live").textContent = T.nres(S.total); }, 500);
  }

  // ---------- filter chips and their menus ----------
  // Each chip: [id, icon, label when off, the label when on, its choices, chosen, pick].
  function chipDefs() {
    const sortLabel = `${T.sortBy}: ${T.sorts[S.sort]}${S.sort === "best" ? "" : ` (${S.desc ? T.desc : T.asc})`}`;
    return [
      { id: "kind", ic: I.doc, label: T.type, on: S.kind > 0 && S.kinds[S.kind], value: `${T.type}: ${S.kinds[S.kind] || ""}`, items: S.kinds, cur: S.kind, pick: (i) => setFilter("kind", i), clear: () => setFilter("kind", 0) },
      { id: "size", ic: I.scale, label: T.size, on: S.size > 0, value: `${T.size}: ${S.sizes[S.size] || ""}`, items: S.sizes, cur: S.size, pick: (i) => setFilter("size", i), clear: () => setFilter("size", 0) },
      { id: "date", ic: I.cal, label: T.date, on: S.date > 0, value: `${T.date}: ${S.dates[S.date] || ""}`, items: S.dates, cur: S.date, pick: (i) => setFilter("date", i), clear: () => setFilter("date", 0) },
      { id: "scope", ic: I.folder, label: `${T.lookIn}: ${T.allDrives}`, on: !!S.scope, value: null, items: [T.allDrives, T.chooseFolder], cur: S.scope ? -1 : 0, pick: (i) => send({ op: i ? "pick_folder" : "all_drives" }), clear: () => send({ op: "all_drives" }) },
      { id: "content", ic: I.text, label: T.inside, toggle: true, on: S.content },
      { id: "sort", ic: I.sort, label: sortLabel, end: true, items: SORTS.map((k) => T.sorts[k]), cur: SORTS.indexOf(S.sort), pick: (i) => setSort(SORTS[i], SORTS[i] === S.sort ? !S.desc : false) },
    ];
  }
  function renderChips() {
    const wraps = chipDefs().map((d) => {
      const chip = h("button", { class: "chip" + (d.on ? " on" : "") + (d.end ? " end" : ""), id: "chip-" + d.id, type: "button", "aria-pressed": d.toggle || d.on ? String(!!d.on) : null, "aria-haspopup": d.items ? "menu" : null, "aria-expanded": d.items ? "false" : null }, icon(d.ic));
      if (d.id === "scope" && S.scope) chip.append(h("span", null, `${T.lookIn}: `), pathEl(S.scope, 48));
      else chip.append(h("span", null, d.on && d.value ? d.value : d.label));
      if (d.items) {
        const caret = icon(d.on && d.clear ? I.x : I.down, "i caret");
        if (d.on && d.clear) { caret.dataset.clear = "1"; caret.setAttribute("aria-label", T.clearFilter); }
        chip.append(caret);
      }
      const wrap = h("span", { class: "chipwrap" + (d.end ? " end" : "") }, chip);
      if (d.items) {
        const menu = h("div", { class: "menu", role: "menu", id: "menu-" + d.id, hidden: true });
        d.items.forEach((label, i) => { const it = h("div", { role: "menuitemradio", tabindex: "-1", "aria-checked": String(i === d.cur) }, label); it.dataset.k = i; menu.append(it); });
        wrap.append(menu);
      }
      chip.addEventListener("click", (e) => {
        if (d.toggle) { S.content = !S.content; renderChips(); q.focus(); updatePlaceholder(); if (S.q.trim()) runSearch(); return; }
        if (d.on && d.clear && e.target.closest("[data-clear]")) { d.clear(); return; }
        toggleMenu("menu-" + d.id);
      });
      wrap.addEventListener("click", (e) => { const it = e.target.closest("[data-k]"); if (it) { closeMenus(); d.pick(+it.dataset.k); } });
      return wrap;
    });
    $("chips").replaceChildren(...wraps, viewButtons());
    updatePlaceholder();
  }
  function updatePlaceholder() { q.placeholder = S.content ? T.phContent : T.ph; }
  function setFilter(key, i) {
    S[key] = i;
    if (key === "kind") S.kindLabel = S.kinds[i] || "";
    closeMenus(); renderChips(); q.focus();
    if (S.q.trim()) runSearch();
  }
  function setSort(sort, desc) {
    S.sort = sort; S.desc = sort === "best" ? false : desc;
    renderChips(); if (S.view === "results") renderCols();
    // Documents are not read again for another order.
    if (S.isContent && S.shown === S.seq && !contentStale()) send({ op: "sort", seq: S.seq, sort: S.sort, desc: S.desc });
    else if (S.q.trim()) runSearch();
  }
  function openMenus() { return [...document.querySelectorAll(".menu")].filter((m) => !m.hidden); }
  function closeMenus() {
    for (const m of openMenus()) m.hidden = true;
    for (const b of document.querySelectorAll('[aria-haspopup="menu"][aria-expanded="true"]')) b.setAttribute("aria-expanded", "false");
  }
  function toggleMenu(id, focusFirst = true) {
    const menu = $(id), open = !menu.hidden;
    closeMenus();
    if (open) return;
    menu.hidden = false;
    const owner = menu.parentElement.querySelector('[aria-haspopup="menu"]');
    if (owner) owner.setAttribute("aria-expanded", "true");
    if (focusFirst) (menu.querySelector('[aria-checked="true"]') || menu.querySelector('[tabindex="-1"]:not([aria-disabled="true"])'))?.focus();
  }
  // Arrow keys, Enter and Esc inside any open menu.
  document.addEventListener("keydown", (e) => {
    const menu = document.activeElement && document.activeElement.closest && document.activeElement.closest(".menu");
    if (!menu || menu.hidden) return;
    const items = [...menu.querySelectorAll('[tabindex="-1"]:not([aria-disabled="true"])')], i = items.indexOf(document.activeElement);
    if (e.key === "ArrowDown") items[(i + 1) % items.length]?.focus();
    else if (e.key === "ArrowUp") items[(i - 1 + items.length) % items.length]?.focus();
    else if (e.key === "Home") items[0]?.focus();
    else if (e.key === "End") items[items.length - 1]?.focus();
    else if (e.key === "Enter" || e.key === " ") document.activeElement.click();
    else if (e.key === "Escape" || e.key === "Tab") { closeMenus(); q.focus(); }
    else return;
    e.preventDefault(); e.stopImmediatePropagation();
  }, true);
  document.addEventListener("mousedown", (e) => { if (!e.target.closest(".chipwrap,.popwrap,.ctx")) closeMenus(); });

  // ---------- the right-click menu ----------
  let ctxRow = null; // { r: the row, at: its list (listAt) }
  let ctxBack = null; // where the focus was before the menu
  function openCtx(i, anchor, listName = "results", at = null) {
    const r = (listName === "results" ? S.rows : S.recent)[i];
    if (!r || r.gone) return;
    ctxRow = { r, at: listAt(listName) };
    ctxBack = document.activeElement;
    const item = (label, key, op, ic) => {
      const it = h("div", { role: "menuitem", tabindex: "-1" }, icon(ic), h("span", null, label), key ? h("kbd", null, key) : null);
      it.addEventListener("click", () => { closeMenus(); runCtx(op); });
      return it;
    };
    // While a Save/Open dialog waits (Ctrl+Alt+G), its move comes first and takes Enter.
    const items = S.qswitch ? [item(T.mSwitch, "Enter", "switch_to", I.folder), item(T.mOpen, null, "open", I.open)] : [item(T.mOpen, "Enter", "open", I.open)];
    items.push(item(T.mReveal, "Ctrl+Enter", "reveal", I.folder), item(T.mCopyFile, "Ctrl+C", "copy_file", I.copy), item(T.mCopyPath, "Ctrl+Shift+C", "copy", I.copy));
    if (listName === "results") items.push(item(T.mCopyAll, null, "copy_all", I.copy));
    items.push(item(T.mScope, null, "scope", I.search), item(T.mSizes, null, "folder_sizes", I.scale));
    if (listName === "results") items.push(item(T.mReplace, null, "replace_text", I.pen));
    if (r.snippet) items.push(item(T.mMatches, "F4", "matches", I.text));
    items.push(h("div", { class: "sep", role: "separator" }), item(T.mRename, "F2", "rename", I.pen), item(T.mRecycle, "Delete", "recycle", I.trash));
    const menu = $("ctx");
    menu.replaceChildren(...items);
    closeMenus();
    menu.hidden = false;
    const rect = at || (anchor ? anchor.getBoundingClientRect() : { left: innerWidth / 2, right: innerWidth / 2, top: innerHeight / 2, bottom: innerHeight / 2 });
    const mw = menu.offsetWidth, mh = menu.offsetHeight;
    const rtl = document.documentElement.dir === "rtl";
    let x = at ? at.left : rtl ? rect.right - mw : rect.left;
    let y = at ? at.top : rect.bottom + 4;
    if (at && rtl) x -= mw;
    x = Math.max(8, Math.min(innerWidth - mw - 8, x));
    if (y + mh > innerHeight - 8) y = Math.max(8, (at ? at.top : rect.top) - mh - 4);
    menu.style.left = x + "px"; menu.style.top = y + "px";
    menu.querySelector('[tabindex="-1"]').focus();
  }
  function runCtx(op) {
    if (!ctxRow) return;
    const { r, at } = ctxRow;
    ctxRow = null;
    if (op === "copy_all") send({ op: "copy_all", seq: at.seq, ver: at.ver });
    else if (op === "rename") renameDialog(r, at.list, at);
    else if (op === "recycle") recycleDialog(r, at.list, at);
    else if (op === "matches") { if (at.ver === S.ver) showMatches(r.id); }
    else if (op === "replace_text") send({ op: "replace_text" });
    else act(op, r, at.list, at);
    if (!$("veil").hidden) return;
    q.focus();
  }

  // ---------- dialogs ----------
  let dlgFor = "", dlgBack = null;
  function dialog(title, body, buttons, tag) {
    // (Opened from the right-click menu: back to where the focus was before it.)
    dlgFor = tag; dlgBack = document.activeElement && document.activeElement.closest("#ctx") ? ctxBack : document.activeElement;
    $("dlg-h").replaceChildren(...[].concat(title));
    $("dlg-body").replaceChildren(...body);
    $("dlg-btns").replaceChildren(...buttons);
    $("veil").hidden = false;
  }
  function closeDialog() {
    $("veil").hidden = true; dlgFor = "";
    (dlgBack && dlgBack.isConnected ? dlgBack : q).focus();
  }
  // The dialogs name the list their row was in when they opened (`at`).
  function renameDialog(r, listName, at = listAt(listName)) {
    const name = plainName(r);
    const input = h("input", { id: "rename", type: "text", dir: "auto", spellcheck: "false", autocomplete: "off", "aria-label": T.renameLabel });
    input.value = name;
    const ok = () => { const v = input.value.trim(); closeDialog(); if (v && v !== name) send({ op: "rename", list: at.list, seq: at.seq, ver: at.ver, id: r.id, name: v }); };
    input.addEventListener("keydown", (e) => { if (e.key === "Enter") { ok(); e.preventDefault(); e.stopPropagation(); } });
    dialog(T.renameH, [h("label", { class: "lbl", for: "rename" }, T.renameLabel), input],
      [btn("primary", I.pen, T.renameOk, ok), btn("", I.x, T.cancel, closeDialog)], "rename");
    input.focus();
    // The name without its extension, ready to type over.
    const dot = r.folder ? -1 : name.lastIndexOf(".");
    input.setSelectionRange(0, dot > 0 ? dot : name.length);
  }
  // The program's own question before the Recycle Bin: Windows asks only
  // when its Recycle Bin is set to (off by default).
  function recycleDialog(r, listName, at = listAt(listName)) {
    if (!r || r.gone) return;
    const name = plainName(r), [before, after] = r.folder ? T.recycleQDir : T.recycleQ;
    const ok = btn("primary", I.trash, T.recycleOk, () => { closeDialog(); act("recycle", r, at.list, at); }, { id: "recycle-ok" });
    dialog(T.recycleH, [h("p", null, before, fileName([[name, false]], r.folder, "dname"), after), h("div", { class: "pv-path" }, pathEl(r.dir, 999)), h("p", { class: "sub" }, T.recycleSub)],
      [ok, btn("", I.x, T.cancel, closeDialog)], "recycle");
    ok.focus();
  }
  function showMatches(id) {
    const r = S.rows[id];
    if (!r || !r.snippet) return;
    const m = S.matches && S.matches.id === id ? S.matches : null;
    if (!m) send({ op: "matches", seq: Math.max(S.shown, 0), ver: S.ver, id });
    const body = [h("div", { class: "pv-path" }, pathEl(r.dir, 999))];
    if (m) {
      body.push(h("p", { class: "sub num" }, T.nMatches(m.count) + (m.lines.length < m.count ? " " + T.firstLines(m.lines.length) : "")), linesEl(m, 1000));
    } else body.push(h("p", { class: "sub" }, T.matchesNone));
    dialog([T.matchesPre, fileName(r.name.map(([t]) => [t, false]), r.folder, "dname")], body, [btn("", I.x, T.close, closeDialog)], "matches:" + id);
    $("dlg-btns").querySelector("button").focus();
  }
  $("veil").addEventListener("mousedown", (e) => { if (e.target === $("veil")) { closeDialog(); e.preventDefault(); } });
  $("veil").addEventListener("keydown", (e) => {
    if (e.key === "Escape") { closeDialog(); e.preventDefault(); e.stopPropagation(); return; }
    if (e.key !== "Tab") return;
    // Keep the focus inside the dialog.
    const f = [...$("dlg").querySelectorAll("input,button")];
    const i = f.indexOf(document.activeElement);
    const next = f[(i + (e.shiftKey ? -1 : 1) + f.length) % f.length];
    if (next) { next.focus(); e.preventDefault(); }
  });

  // ---------- history ----------
  function renderHistory() {
    const menu = $("hist");
    const item = (cls, ic, label, sub, onClick, disabled) => {
      const it = h("div", { role: "menuitem", tabindex: "-1", class: cls, "aria-disabled": disabled ? "true" : null }, icon(ic), h("bdi", { class: "lbl" }, label), sub ? h("span", { class: "sub" }, sub) : null);
      if (!disabled) it.addEventListener("click", () => { closeMenus(); onClick(); });
      return it;
    };
    const kids = [];
    if (S.history.saved.length) {
      kids.push(h("div", { class: "mh", role: "presentation" }, T.savedH));
      for (const s of S.history.saved) kids.push(item("", I.star, s.name, [s.content ? T.savedInside : "", s.scoped ? T.savedScoped : ""].filter(Boolean).join(" · "), () => send({ op: "saved", id: s.id, name: s.name })));
      kids.push(h("div", { class: "sep", role: "separator" }));
    }
    kids.push(h("div", { class: "mh", role: "presentation" }, S.history.recent.length ? T.recentH : T.recentEmpty));
    for (const r of S.history.recent) kids.push(item("", I.clock, r, null, () => { closeSettings(); q.value = r; runSearch(); q.focus(); }));
    kids.push(h("div", { class: "sep", role: "separator" }));
    kids.push(item("act", I.star, T.saveCurrent, null, () => send({ op: "save_search", q: S.q, kind: S.kind, content: S.content }), !S.q.trim()));
    kids.push(item("act", I.x, T.clearRecent, null, () => send({ op: "clear_recent" }), !S.history.recent.length));
    kids.push(item("act", I.trash, T.clearSaved, null, () => send({ op: "clear_saved" }), !S.history.saved.length));
    menu.replaceChildren(...kids);
  }
  function openHistory() { renderHistory(); toggleMenu("hist"); if (!$("hist").hidden) $("hist").querySelector('[tabindex="-1"]:not([aria-disabled="true"])')?.focus(); }
  $("hist-btn").addEventListener("click", openHistory);

  // ---------- settings ----------
  function openSettings() {
    closeMenus();
    S.prevView = S.view === "settings" ? S.prevView : S.view;
    setView("settings");
    renderSettings(); renderBanner(); renderStatus();
    $("settings").querySelector("button")?.focus();
  }
  function closeSettings() {
    if (S.view !== "settings") return;
    S.view = S.prevView || "empty";
    render(true);
    q.focus();
  }
  function renderSettings() {
    const st = S.settings;
    const box = $("settings");
    if (!st) { box.replaceChildren(); return; }
    const sw = (key, label, sub) => {
      const on = !!st[key];
      const b = h("button", { class: "sw", type: "button", role: "switch", "aria-checked": String(on), id: "set-" + key, "aria-label": label }, h("span", { class: "knob" }));
      b.addEventListener("click", () => send({ op: "set", key, on: !on }));
      return h("div", { class: "srow" }, h("label", { for: "set-" + key }, h("span", null, label), sub ? h("small", null, sub) : null), b);
    };
    const section = (title, ...rows) => h("section", { class: "sbox" }, h("h3", null, title), ...rows);
    const langBtn = (code, label) => {
      const on = st.next_lang === code;
      const b = h("button", { class: "seg" + (on ? " on" : ""), type: "button", role: "radio", "aria-checked": String(on), lang: code, dir: code === "ar" ? "rtl" : "ltr" }, label);
      b.addEventListener("click", () => send({ op: "lang", lang: code }));
      return b;
    };
    const lang = section(T.sLang, h("div", { class: "segs", role: "radiogroup", "aria-label": T.sLang }, langBtn("ar", "العربية"), langBtn("en", "English")));
    if (st.next_lang !== st.lang) lang.append(h("div", { class: "snote" }, h("span", null, T.sLangNote), btn("primary", I.open, T.restart, () => send({ op: "restart" }))));
    const reindex = btn("", I.search, T.reindex, () => send({ op: "reindex" }));
    const back = btn("", I.back, T.back, closeSettings, { class: "btn back" });
    box.replaceChildren(
      h("div", { class: "shead" }, back, h("h2", null, T.settings)),
      lang,
      section(T.sDisplay, sw("digits", T.sDigits),
        (() => {
          const on = compactOn();
          const b = h("button", { class: "sw", type: "button", role: "switch", "aria-checked": String(on), id: "set-compact", "aria-label": T.compact }, h("span", { class: "knob" }));
          b.addEventListener("click", () => setPref("compact", !compactOn()));
          return h("div", { class: "srow" }, h("label", { for: "set-compact" }, h("span", null, T.compact), h("small", null, T.compactSub)), b);
        })()),
      section(T.sSearch, sw("hide_system", T.sHideSystem, T.sHideSystemSub), sw("content_index", T.sContentIndex, T.sContentIndexSub), sw("read_cloud_files", T.sReadCloud), sw("auto_index", T.sAutoIndex), h("div", { class: "srow" }, reindex)),
      section(T.sAnywhere, sw("hotkey_quick", T.sHotkey), sw("floating", T.sFloating), sw("tray_icon", T.sTray)),
      section(T.sStartup, sw("autostart", T.sAutostart)),
      section(T.sMore, h("div", { class: "srow" }, h("label", { for: "options-menu" }, h("span", null, T.sMoreRow), h("small", null, T.sMoreSub)), btn("", I.dots, T.moreBtn, () => send({ op: "options_menu" }), { id: "options-menu" })),
        h("div", { class: "srow" }, h("label", { for: "report-problem" }, h("span", null, T.sReportRow), h("small", null, T.sReportSub)), btn("", I.flag, T.reportBtn, () => send({ op: "report_problem" }), { id: "report-problem" }))),
      section(T.sAbout, h("p", { class: "num" }, digits(T.version(st.version))), h("p", { class: "muted" }, T.trust)),
    );
  }
  $("set-btn").addEventListener("click", () => (S.view === "settings" ? closeSettings() : openSettings()));

  // ---------- input ----------
  q.addEventListener("input", onInput);
  $("stop").addEventListener("click", () => { stop(); q.focus(); });
  // Clicking a suggestion or a recent file keeps the caret in the search box;
  // clicking a result puts the focus on the list, as in the classic window
  // (Delete works only there). Typing there goes back to the box.
  for (const el of [list, $("recent"), $("sugg"), $("cols")]) el.addEventListener("mousedown", (e) => { if (e.target.closest(".row,.sg,.colbtn")) e.preventDefault(); });
  const focusList = () => list.focus({ preventScroll: true });
  list.addEventListener("click", (e) => { const r = e.target.closest(".row"); if (r) { focusList(); select(+r.dataset.i); } });
  list.addEventListener("dblclick", (e) => { const r = e.target.closest(".row"); if (r) act("open", S.rows[+r.dataset.i]); });
  list.addEventListener("contextmenu", (e) => { const r = e.target.closest(".row"); e.preventDefault(); if (r) { focusList(); select(+r.dataset.i, false); openCtx(+r.dataset.i, null, "results", { left: e.clientX, top: e.clientY }); } });
  list.addEventListener("scroll", () => { if (list.scrollTop + list.clientHeight > list.scrollHeight - FIRST_PAGE_GUARD) askMore(); });
  $("recent").addEventListener("click", (e) => { const r = e.target.closest(".row"); if (r) act("open", S.recent[+r.dataset.i], "recent"); });
  // Dragging a row out (to Explorer, a mail, a chat) is the program's own
  // drag of the file, as in the classic list: a press on a row that moves a
  // few pixels with the button down asks for it (once per press). (The
  // rows' mousedown keeps the focus in the box, so the page itself never
  // starts a drag.)
  // The click that ends a drag opens nothing.
  let press = null, dragged = false;
  for (const [el, listName] of [[list, "results"], [$("recent"), "recent"]]) {
    el.addEventListener("mousedown", (e) => {
      const r = e.button === 0 && e.target.closest(".row");
      press = r ? { x: e.clientX, y: e.clientY, i: +r.dataset.i, listName } : null;
      dragged = false;
    });
  }
  document.addEventListener("mousemove", (e) => {
    if (!press || !(e.buttons & 1)) { press = null; return; }
    if (Math.abs(e.clientX - press.x) + Math.abs(e.clientY - press.y) < 6) return;
    const p = press; press = null; dragged = true;
    setTimeout(() => { dragged = false; }, 300);
    act("drag", (p.listName === "recent" ? S.recent : S.rows)[p.i], p.listName);
  });
  document.addEventListener("mouseup", () => { press = null; });
  document.addEventListener("click", (e) => { if (dragged) { dragged = false; e.stopPropagation(); e.preventDefault(); } }, true);
  // The program's drag ends without a mouseup here (Windows' drag loop takes
  // it), or does not start: the flag is forgotten on the next press, when
  // the window loses the focus, and shortly after the drag anyway.
  document.addEventListener("mousedown", () => { if (!press) dragged = false; }, true);
  window.addEventListener("blur", () => { dragged = false; press = null; });
  $("recent").addEventListener("contextmenu", (e) => { const r = e.target.closest(".row"); e.preventDefault(); if (r) openCtx(+r.dataset.i, null, "recent", { left: e.clientX, top: e.clientY }); });
  $("sugg").addEventListener("click", (e) => { const s = e.target.closest(".sg"); if (s) applySuggestion(S.sugg[+s.dataset.k]); });
  $("tips").addEventListener("click", (e) => { const c = e.target.closest("code"); if (c) { q.value = c.textContent; q.focus(); runSearch(); } });
  $("cols").addEventListener("click", (e) => {
    const b = e.target.closest(".colbtn");
    if (!b) return;
    // As the classic column headers: ascending, then descending, then best match.
    const key = b.dataset.sort;
    if (S.sort !== key) setSort(key, false); else if (!S.desc) setSort(key, true); else setSort("best", false);
  });
  // Nothing else of the browser's own menu.
  document.addEventListener("contextmenu", (e) => { if (!e.target.closest("input")) e.preventDefault(); });

  document.addEventListener("keydown", (e) => {
    if (!$("veil").hidden) return;
    const ctrl = e.ctrlKey || e.metaKey;
    if (ctrl && (e.key === "k" || e.key === "K" || e.key === "f" || e.key === "F") || e.key === "F3") { closeSettings(); q.focus(); q.select(); e.preventDefault(); return; }
    if (ctrl && (e.key === "h" || e.key === "H")) { openHistory(); e.preventDefault(); return; }
    if (ctrl && e.key === ",") { S.view === "settings" ? closeSettings() : openSettings(); e.preventDefault(); return; }
    if (ctrl && (e.key === "t" || e.key === "T")) { toggleGrid(); e.preventDefault(); return; }
    // Zoom (the browser's own keys are off): Ctrl and + / - / 0, kept for next time.
    if (ctrl && (e.key === "+" || e.key === "=" || e.key === "-" || e.key === "0")) { zoomBy(e.key === "0" ? 0 : e.key === "-" ? -1 : 1); e.preventDefault(); return; }
    if (e.key === "Escape") {
      if (openMenus().length) closeMenus();
      else if (S.qswitch) send({ op: "switch_cancel" });
      else if (S.view === "settings") closeSettings();
      else if (S.running) stop();
      else if (q.value) { q.value = ""; onInput(); q.focus(); }
      else send({ op: "hide" });
      e.preventDefault();
      return;
    }
    if (S.view === "settings") return;
    const inBox = document.activeElement === q || document.activeElement === document.body;
    // Down in an empty box opens the history, as in the classic window.
    if (e.key === "ArrowDown" && inBox && !q.value.trim()) { openHistory(); e.preventDefault(); return; }
    if (e.key === "Enter" && inBox && S.content && contentStale() && q.value.trim()) { runSearch(); e.preventDefault(); return; }
    if (S.view === "nores" && S.sugg.length) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        S.ssel = (S.ssel + (e.key === "ArrowDown" ? 1 : S.sugg.length - 1)) % S.sugg.length;
        renderNoResults(); e.preventDefault();
      } else if (e.key === "Enter" && inBox) { applySuggestion(S.sugg[S.ssel]); e.preventDefault(); }
      return;
    }
    if (S.view !== "results" || !S.rows.length) return;
    const inList = document.activeElement === list;
    // Typing in the list goes on in the search box.
    if (inList && !ctrl && !e.altKey && (e.key.length === 1 || e.key === "Backspace")) { q.focus(); return; }
    if (inList && e.key === "ArrowUp" && S.sel === 0) { q.focus(); e.preventDefault(); return; }
    const row = S.rows[S.sel];
    const rowH = (list.querySelector(".row") || { offsetHeight: 56 }).offsetHeight || 56;
    // In the grid, Up and Down move by a row of tiles, Left and Right by one
    // (in the list only; in the box they move the caret), in reading order.
    const cols = gridOn() ? Math.max(1, getComputedStyle(list).gridTemplateColumns.split(" ").length) : 1;
    const page = Math.max(1, Math.floor(list.clientHeight / rowH) - 1) * cols;
    const moves = { ArrowDown: cols, ArrowUp: -cols, PageDown: page, PageUp: -page };
    if (gridOn() && inList) {
      const rtl = document.documentElement.dir === "rtl";
      moves.ArrowRight = rtl ? -1 : 1; moves.ArrowLeft = rtl ? 1 : -1;
    }
    if (e.key in moves && (inBox || inList)) { select(Math.max(0, Math.min(S.rows.length - 1, S.sel + moves[e.key]))); e.preventDefault(); return; }
    if (ctrl && (e.key === "Home" || e.key === "End")) { select(e.key === "Home" ? 0 : S.rows.length - 1); e.preventDefault(); return; }
    // A Save/Open dialog waits (Ctrl+Alt+G): Enter moves it to the folder of
    // the selected result (the top one from the box), as in the classic window.
    if (e.key === "Enter" && S.qswitch && !ctrl && (inBox || inList)) { switchTo(row); e.preventDefault(); return; }
    // Enter in the box moves to the list (as the classic window, where it
    // searched); there Enter opens. Ctrl+Enter opens the folder from both.
    if (e.key === "Enter" && inBox && !ctrl) { focusList(); select(S.sel); e.preventDefault(); return; }
    if (e.key === "Enter" && (inBox || inList)) { act(ctrl ? "reveal" : "open", row); e.preventDefault(); return; }
    // The keys of the classic list (in the box, only when no text is selected there).
    const noSelection = inList || q.selectionStart === q.selectionEnd;
    if (ctrl && (e.key === "c" || e.key === "C") && noSelection) { act(e.shiftKey ? "copy" : "copy_file", row); e.preventDefault(); return; }
    if (e.key === "F2") { if (!row.gone) renameDialog(row, "results"); e.preventDefault(); return; }
    if (e.key === "F4") { showMatches(S.sel); e.preventDefault(); return; }
    // Delete only in the list, as in the classic window (in the box it edits
    // the text), and only after the program's own question.
    if (e.key === "Delete" && inList && !ctrl && !e.shiftKey && !e.altKey) { recycleDialog(row, "results"); e.preventDefault(); return; }
    if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) { openCtx(S.sel, $("r" + S.sel)); e.preventDefault(); }
  });

  // ---------- start ----------
  for (const el of document.querySelectorAll("[data-i18n]")) el.textContent = T[el.dataset.i18n];
  for (const el of document.querySelectorAll("[data-i18n-placeholder]")) el.placeholder = T[el.dataset.i18nPlaceholder];
  for (const el of document.querySelectorAll("[data-i18n-label]")) el.setAttribute("aria-label", T[el.dataset.i18nLabel]);
  for (const el of document.querySelectorAll("[data-i18n-title]")) el.title = T[el.dataset.i18nTitle];
  $("tips").replaceChildren(...T.tips.map(([c, s]) => h("div", { class: "tip" }, h("code", { dir: "auto" }, c), h("span", null, s))));
  setView("empty");
  applyView();
  applyZoom();
  applyCols();
  renderChips();
  renderRecent();
  renderStatus();
  q.focus();
  send({ op: "hello" });
})();
