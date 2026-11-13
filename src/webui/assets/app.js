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
  // The interface language (src/i18n.rs page_data): its code, direction,
  // the translations of this file's texts and the languages to offer.
  // Arabic and English are the originals below; every other language
  // starts from English (LANG) and replaces what its file translates.
  const I18N = (() => {
    try { return JSON.parse(document.getElementById("i18n").textContent) || {}; } catch (e) { return {}; }
  })();
  const CODE = document.documentElement.lang || "ar";
  const LANG = CODE === "ar" ? "ar" : "en";
  const FIRST_PAGE_GUARD = 300; // px from the bottom that asks for more rows

  // ---------- strings ----------
  // T: the texts of the interface language. A translated language is
  // T.en with every text its file translates (src/i18n.rs; nested objects
  // and lists key by key). A function's translation is not code to run:
  // it is read by compile() below, a small expression language
  // (parameters, texts, `${…}` templates, [lists], ?:, comparisons, + - * / %,
  // num(), T.key),
  // and a translation it can't read, or one that fails, gives English.
  function localize(tables) {
    const base = tables[LANG];
    if (LANG !== "en" || CODE === "en" || !I18N.js || typeof I18N.js !== "object") return base;
    const merge = (en, tr) => {
      if (tr === undefined || tr === null) return en;
      if (typeof en === "function") return compile(en, tr);
      if (Array.isArray(en)) return en.map((x, i) => merge(x, Array.isArray(tr) ? tr[i] : tr && tr[i]));
      if (en && typeof en === "object") {
        const o = {};
        for (const k of Object.keys(en)) o[k] = merge(en[k], Object.prototype.hasOwnProperty.call(tr, k) ? tr[k] : undefined);
        return o;
      }
      return typeof tr === "string" && tr.trim() ? tr : en;
    };
    return merge(base, I18N.js);
  }

  // The translation `src` of English function `en`: "FN:(a, b) => expr"
  // or a template text with ${a} places. Returns a function with en's
  // parameters, falling back to en on any problem.
  function compile(en, src) {
    if (typeof src !== "string" || !src.trim()) return en;
    const params = (s) => { const m = /^\s*(?:\(([^)]*)\)|([A-Za-z_$][\w$]*))\s*=>/.exec(s); return m ? (m[1] !== undefined ? m[1] : m[2]).split(",").map((p) => p.trim()).filter(Boolean) : null; };
    const want = params(String(en));
    if (!want) return en;
    let body = src, isTemplate = true;
    if (src.startsWith("FN:")) {
      const p = params(src.slice(3));
      if (!p || p.join() !== want.join()) return en;
      body = src.slice(src.indexOf("=>") + 2);
      isTemplate = false;
    }
    let i = 0;
    const fail = () => { throw new Error("i18n"); };
    const ws = () => { while (i < body.length && /\s/.test(body[i])) i++; };
    const eat = (t) => { ws(); if (body.startsWith(t, i)) { i += t.length; return true; } return false; };
    // Each parse function returns an evaluator: (args) => value.
    function expr() {
      const c = or();
      if (eat("?")) { const a = expr(); if (!eat(":")) fail(); const b = expr(); return (v) => (c(v) ? a(v) : b(v)); }
      return c;
    }
    function or() { let l = and(); while (eat("||")) { const a = l, b = and(); l = (v) => a(v) || b(v); } return l; }
    function and() { let l = cmp(); while (eat("&&")) { const a = l, b = cmp(); l = (v) => a(v) && b(v); } return l; }
    function cmp() {
      const l = add();
      const ops = { "===": (a, b) => a === b, "!==": (a, b) => a !== b, "==": (a, b) => a == b, "!=": (a, b) => a != b, "<=": (a, b) => a <= b, ">=": (a, b) => a >= b, "<": (a, b) => a < b, ">": (a, b) => a > b };
      for (const op of Object.keys(ops)) if (eat(op)) { const r = add(), f = ops[op]; return (v) => f(l(v), r(v)); }
      return l;
    }
    function add() {
      let l = mul();
      for (;;) {
        if (eat("+")) { const a = l, b = mul(); l = (v) => a(v) + b(v); } else if (eat("-")) { const a = l, b = mul(); l = (v) => a(v) - b(v); } else return l;
      }
    }
    // % for plural rules (n % 10 === 1), * and /.
    function mul() {
      let l = unary();
      for (;;) {
        if (eat("%")) { const a = l, b = unary(); l = (v) => a(v) % b(v); } else if (eat("*")) { const a = l, b = unary(); l = (v) => a(v) * b(v); } else if (eat("/")) { const a = l, b = unary(); l = (v) => a(v) / b(v); } else return l;
      }
    }
    function unary() { if (eat("!")) { const a = unary(); return (v) => !a(v); } return primary(); }
    function template(end) {
      const parts = [];
      let text = "";
      while (i < body.length) {
        const ch = body[i];
        if (ch === "\\") { text += body[i + 1] === "n" ? "\n" : body[i + 1] || ""; i += 2; continue; }
        if (ch === "$" && body[i + 1] === "{") {
          i += 2;
          if (text) { const t = text; parts.push(() => t); text = ""; }
          const e = expr();
          if (!eat("}")) fail();
          parts.push((v) => { const x = e(v); return typeof x === "number" ? num(x) : String(x); });
          continue;
        }
        if (end && ch === end) { i++; if (text) { const t = text; parts.push(() => t); } return (v) => parts.map((p) => p(v)).join(""); }
        text += ch; i++;
      }
      if (end) fail();
      if (text) { const t = text; parts.push(() => t); }
      return (v) => parts.map((p) => p(v)).join("");
    }
    function primary() {
      ws();
      const ch = body[i];
      if (ch === "(") { i++; const e = expr(); if (!eat(")")) fail(); return e; }
      if (ch === '"' || ch === "'") {
        let s = ""; i++;
        while (i < body.length && body[i] !== ch) { if (body[i] === "\\") i++; s += body[i] === undefined ? "" : body[i]; i++; }
        if (body[i] !== ch) fail();
        i++;
        return () => s;
      }
      if (ch === "`") { i++; return template("`"); }
      // A list, [a, b, …] (helpKeys gives rows of [key, text]).
      if (ch === "[") {
        i++;
        const items = [];
        if (!eat("]")) { do { if (eat("]")) { i--; break; } items.push(expr()); } while (eat(",")); if (!eat("]")) fail(); }
        return (v) => items.map((x) => x(v));
      }
      const n = /^\d+(\.\d+)?/.exec(body.slice(i));
      if (n) { i += n[0].length; const x = Number(n[0]); return () => x; }
      const id = /^[A-Za-z_][\w]*/.exec(body.slice(i));
      if (!id) fail();
      i += id[0].length;
      let get;
      if (id[0] === "T") {
        if (!eat(".")) fail();
        ws();
        const k = /^[A-Za-z_][\w]*/.exec(body.slice(i));
        if (!k) fail();
        i += k[0].length;
        get = () => T[k[0]];
      } else if (id[0] === "num") {
        get = () => num;
      } else {
        const at = want.indexOf(id[0]);
        if (at < 0) fail();
        get = (v) => v[at];
      }
      if (eat("(")) {
        const args = [];
        if (!eat(")")) { do args.push(expr()); while (eat(",")); if (!eat(")")) fail(); }
        return (v) => { const f = get(v); if (typeof f !== "function") fail(); return f(...args.map((a) => a(v))); };
      }
      return get;
    }
    let run;
    try {
      if (isTemplate) run = template(null);
      else { run = expr(); ws(); if (i !== body.length) fail(); }
    } catch (e) { return en; }
    return (...v) => { try { const r = run(v); return r === undefined || r === null ? en(...v) : r; } catch (e) { return en(...v); } };
  }

  const T = localize({
    ar: {
      ph: "ابحث باسم الملف…", phContent: "ابحث داخل المستندات ثم اضغط Enter…", type: "النوع", size: "الحجم", date: "التاريخ",
      filters: "عوامل التصفية", lookIn: "البحث في", allDrives: "كل الأقراص", chooseFolder: "اختيار مجلد…", clearFilter: "إزالة",
      inside: "داخل المستندات", sortBy: "الترتيب",
      sorts: { best: "الأنسب أولاً", name: "الاسم", path: "المسار", size: "الحجم", date: "تاريخ التعديل" },
      asc: "تصاعدي", desc: "تنازلي",
      cName: "الاسم والمسار", cSize: "الحجم", cDate: "تاريخ التعديل", resize: (c) => `تغيير عرض عمود «${c}»`,
      results: "النتائج", preview: "المعاينة",
      recent: "ملفات فتحتها مؤخراً", recentNone: "الملفات التي تفتحها تظهر هنا.", tryThese: "جرّب هذه",
      trust: "ملفاتك وفهرسها يبقيان على جهازك ولا نجمع بيانات استخدام. يتصل البرنامج بالإنترنت فقط ليتحقق من التحديثات (GitHub) ولأسعار العملات؛ والبحث من الهاتف يعمل داخل شبكة بيتك فقط.",
      trustStore: "ملفاتك وفهرسها يبقيان على جهازك ولا نجمع بيانات استخدام. يتصل البرنامج بالإنترنت فقط لأسعار العملات، وتصله التحديثات من Microsoft Store.",
      tips: [["تقرير .pdf", "ملفات PDF فقط"], ["حجم:>10م", "أكبر من 10 ميغابايت"], ["تاريخ:هذا_الأسبوع", "عُدّلت هذا الأسبوع"], ['"عقد إيجار"', "العبارة كما هي"]],
      tipsArab: [["جذر: كتاب", "مكتبة، الكاتب، كتابات"], ["hgjrvdv", "لوحة المفاتيح الخطأ ← «التقرير»"]],
      tipFold: ["cafe", "يجد «café» أيضاً"],
      kNav: "تنقل", kOpen: "فتح", kFolder: "المجلد", kMenu: "المزيد",
      open: "فتح", openFolder: "فتح المجلد", copyPath: "نسخ المسار", more: "إجراءات أخرى",
      fDetails: "تفاصيل الملف", kLabel: "اختصارات لوحة المفاتيح", fType: "النوع", fSize: "الحجم", fMod: "تاريخ التعديل", fMatches: "التطابقات", folder: "مجلد", file: "ملف", fileOf: (x) => `ملف ${x}`,
      // Counts read «label: n» (no noun that must agree with the number), as the classic window.
      nres: (n) => (n === 0 ? "لا نتائج" : `عدد النتائج: ${num(n)}`),
      nMatches: (n) => `عدد التطابقات: ${num(n)}`,
      count: (n, ms) => (n ? `${T.nres(n)} في ${num(ms)} ملّي ثانية` : T.nres(0)),
      files: (n) => `الفهرس محدَّث · عدد الملفات: ${num(n)}`,
      noresH: (q) => `لا توجد نتائج لـ «${q}»`,
      noresSub: (n, kind, scope) => `بحثنا في أسماء الملفات ${scope ? `داخل ${scope}` : "في كل الأقراص"}${kind ? `، ضمن «${kind}» فقط` : ""} (عددها: ${num(n)}).`,
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
      rateText: "هل يفيدك InstaFind؟ تقييمك في المتجر يساعد غيرك على إيجاده.", rateOk: "قيّم InstaFind في المتجر", rateLater: "ليس الآن", rateRegion: "التقييم في المتجر",
      mSwitch: "فتح مجلدها في نافذة الحفظ/الفتح", qsCancel: "إلغاء النقل", qsNone: "لا توجد نتيجة لنقل النافذة إلى مجلدها. غيّر البحث، أو اضغط Esc للإلغاء.", mOpenWith: "فتح باستخدام…", mProps: "خصائص",
      // dialogs
      renameH: "تغيير الاسم", renameLabel: "الاسم الجديد", renameOk: "تغيير الاسم", cancel: "إلغاء", close: "إغلاق",
      recycleH: "نقل إلى سلة المحذوفات", recycleQ: ["هل تريد نقل «", "» إلى سلة المحذوفات؟"], recycleQDir: ["هل تريد نقل المجلد «", "» وكل ما فيه إلى سلة المحذوفات؟"],
      recycleSub: "يمكنك استعادته من سلة المحذوفات.", recycleOk: "نقل إلى سلة المحذوفات",
      matchesH: (name) => `كل التطابقات — ${name}`, line: (n) => `سطر ${num(n)}`, firstLines: (n) => `(أول الأسطر فقط، عددها: ${num(n)})`,
      matchesNone: "يُعرض هنا كل سطر يطابق البحث.",
      gone: "نُقل إلى سلة المحذوفات", offline: "القرص غير متصل",
      // history
      history: "السجل", historyKey: "السجل (Ctrl+H)", savedH: "البحوث المحفوظة", recentH: "عمليات البحث الأخيرة", recentEmpty: "لا توجد عمليات بحث سابقة بعد",
      saveCurrent: "حفظ البحث الحالي", clearRecent: "مسح السجل والملفات الأخيرة", clearSaved: "حذف كل البحوث المحفوظة",
      savedInside: "داخل المستندات", savedScoped: "في مجلد",
      // settings
      settings: "الإعدادات", settingsKey: "الإعدادات (Ctrl+,)", back: "رجوع إلى البحث",
      sLang: "اللغة", sLangNote: "ستظهر الواجهة باللغة المختارة بعد إعادة تشغيل البرنامج.", restart: "إعادة التشغيل الآن",
      sLangMachine: "(ترجمة آلية)", sLangHelp: "ساعد في تحسين هذه الترجمة",
      sDisplay: "العرض", sDigits: "عرض الأرقام بالعربية (٠١٢٣)",
      app: "تطبيق", appsLabel: "برامج بهذا الاسم",
      grid: "عرض المصغرات (Ctrl+T)",
      clearQ: "مسح البحث", filtersBtn: "التصفية", clearFilters: "إزالة كل عوامل التصفية", ixContents: (n) => `فهرسة محتوى المستندات · ${n}٪`,
      theme: "السمة", themeSub: "«تلقائي» يتبع الوضع الفاتح أو الداكن في ويندوز", themes: { auto: "تلقائي", light: "فاتح", dark: "داكن", midnight: "ليلي أزرق", kiwi: "كيوي" },
      compact: "عرض مضغوط", compactSub: "سطر واحد لكل نتيجة، فتظهر نتائج أكثر", showPreview: "إظهار المعاينة", hidePreview: "إخفاء المعاينة",
      sMore: "المزيد", sMoreRow: "خيارات أخرى", sMoreSub: "التحقق من التحديثات، الدعم والملاحظات، أدوات مثل الملفات المكررة وأحجام المجلدات، وإعدادات البحث من أي مكان", moreBtn: "فتح قائمة الخيارات…",
      sHelp: "التحديثات والمساعدة", sUpdRow: "التحديثات", sUpdSub: (v) => `الإصدار المثبّت ${v}. يتحقق البرنامج من وجود إصدار أحدث ويعرض عليك تثبيته.`, updBtn: "التحقق من التحديثات…", updChecking: "جارٍ التحقق من التحديثات…",
      sUpdSubStore: (v) => `الإصدار المثبّت ${v}. تصل التحديثات من Microsoft Store.`, updBtnStore: "التحديثات من Microsoft Store…",
      sReportRow: "الإبلاغ عن مشكلة", sReportSub: "تعرض ما سيُشارَك قبل أي شيء، ثم ترسله أنت بالبريد أو من صفحة GitHub (وهي عامة يراها أي أحد). لا يُرسل شيء تلقائياً، وأسماء الملفات والمسارات تُحذف.", reportBtn: "الإبلاغ عن مشكلة…",
      sSearch: "البحث والفهرسة", sHideSystem: "إخفاء ملفات النظام", sHideSystemSub: "مجلد ويندوز والملفات المؤقتة وملفات الذاكرة المخبأة",
      sContentIndex: "فهرسة محتوى المستندات في الخلفية", sContentIndexSub: "بحث أسرع داخل الملفات، ويشمل النص في الصور والملفات الممسوحة",
      sReadCloud: "قراءة محتوى الملفات الموجودة على السحابة فقط (يُنزّلها من الإنترنت)",
      sReadPrograms: "اقرأ داخل ملفات البرامج والنظام أيضاً",
      sAutoIndex: "تحديث الفهرس تلقائياً عند تغيّر الملفات", reindex: "تحديث الفهرس الآن",
      sAnywhere: "البحث من أي مكان", sHotkey: (k) => `${k} يفتح خانة البحث الصغيرة؛ عند إيقافه يفتح هذه النافذة`,
      sHotkeyKeys: "اختصار فتح البحث", sHotkeyKeysSub: "يعمل من أي برنامج. اختر غيره إن كان برنامج آخر يستعمله.",
      sHotkeyTaken: (k) => `الاختصار ${k} يستخدمه برنامج آخر، فلا يعمل الآن. اختر اختصاراً غيره:`,
      sFloating: "خانة بحث شفافة فوق الساعة", sTray: "أيقونة بجانب الساعة",
      sStartup: "التشغيل", sAutostart: "التشغيل مع ويندوز (في الخلفية)",
      sPhone: "البحث من الهاتف", sPhoneRow: "البحث من الهاتف (شبكة البيت)", sPhoneSub: "ابحث في ملفات هذا الحاسوب من متصفح هاتفك المتصل بشبكة Wi-Fi نفسها. للقراءة فقط، ويعمل في شبكة البيت فقط.",
      sPhoneOpen: "افتح في متصفح الهاتف:", sPhonePin: "واكتب الرمز السري:", sPhoneQr: "أو امسح هذا الرمز بكاميرا الهاتف ليفتح البحث مباشرة دون الرمز السري. لا تُطلع عليه أحداً.",
      sPhoneQrLabel: "رمز QR يفتح البحث من الهاتف", sPhoneOff: "متوقف. شغّله لترى العنوان والرمز.", sPhoneWait: "جارٍ التحقق من الشبكة…",
      sPhoneIdle: "توقّف وحده بعد ساعتين دون أي طلب من الهاتف، للأمان. شغّله من جديد حين تحتاجه.", sPhoneAllow: "السماح بهذه الشبكة", sPhoneMore: "إعدادات أخرى…", sPhoneMoreSub: "المنفذ، والسماح بتنزيل الملفات، ورمز ورابط جديدان",
      sAbout: "حول البرنامج", version: (v) => `بحث سريع (InstaFind) — الإصدار ${v}`,
      // «كيف أبحث؟» (the ? button and F1): examples to click, and the keys.
      helpH: "كيف أبحث؟", helpKey: "كيف أبحث؟ (F1)", helpMore: "كل طرق البحث…", helpSub: "انقر مثالاً ليُكتب في خانة البحث. تعمل الأمثلة بالعربية والإنجليزية، والأرقام العربية مقبولة.", helpSubArab: "",
      helpEx: [["تقرير المبيعات", "الاسم فيه الكلمتان معاً"], ['"عقد إيجار"', "العبارة كما هي"], ["تقرير .pdf", "ملفات PDF فقط (أو ext:pdf)"], ["ext:docx,xlsx", "أكثر من نوع"],
        ["صورة|فيديو", "إحدى الكلمتين"], ["عقد -قديم", "دون ما فيه «قديم» (أو !قديم)"], ["حجم:>10م", "أكبر من 10 ميغابايت"], ["تاريخ:هذا_الأسبوع", "عُدّلت هذا الأسبوع"],
        ["dm:2026-05", "عُدّلت في أيار 2026"], ["كلمة:عقد", "الكلمة كاملة: «عقد» لا «العقود»"], ['"عقد إيجار"~5', "الكلمتان وبينهما 5 كلمات على الأكثر"], ["مجلد: صور", "المجلدات فقط"],
        ["تقريب: تقريز", "يتحمّل خطأً إملائياً"]],
      helpExArab: [["جذر: كتاب", "مكتبة، الكاتب، كتابات"], ["size:>1gb", "بالإنجليزية أيضاً: أكبر من 1 غيغابايت"], ["dm:thisweek", "بالإنجليزية أيضاً: هذا الأسبوع"]],
      helpInside: "للبحث داخل المستندات (PDF وWord وExcel والصور): اضغط «داخل المستندات» ثم Enter.",
      helpKeysH: "لوحة المفاتيح",
      helpKeys: (hk) => [[hk, "خانة البحث الصغيرة من أي برنامج"], ["Ctrl+Alt+G", "في نافذة حفظ أو فتح: الانتقال إلى مجلد نتيجة"], ["Enter", "فتح الملف"], ["Ctrl+Enter", "فتح مجلده"], ["↓", "إلى النتائج (وفي خانة فارغة: السجل)"],
        ["Ctrl+C", "نسخ الملف"], ["Ctrl+Shift+C", "نسخ المسار"], ["F2", "تغيير الاسم"], ["Delete", "إلى سلة المحذوفات"], ["F4", "كل التطابقات داخل الملف"], ["Ctrl+H", "السجل"], ["Ctrl+T", "المصغرات"], ["Ctrl+,", "الإعدادات"], ["F1", "هذه البطاقة"]],
    },
    en: {
      ph: "Search file names…", phContent: "Search inside documents, then press Enter…", type: "Type", size: "Size", date: "Date",
      filters: "Filters", lookIn: "Look in", allDrives: "All drives", chooseFolder: "Choose folder…", clearFilter: "Remove",
      inside: "Inside documents", sortBy: "Sort",
      sorts: { best: "Best match first", name: "Name", path: "Path", size: "Size", date: "Modified" },
      asc: "ascending", desc: "descending",
      cName: "Name and path", cSize: "Size", cDate: "Modified", resize: (c) => `Resize the “${c}” column`,
      results: "Results", preview: "Preview",
      recent: "Recently opened files", recentNone: "Files you open show up here.", tryThese: "Try these",
      trust: "Your files and their index stay on your PC; no usage data is collected. InstaFind goes online only to check for updates (GitHub) and fetch exchange rates; phone search works only on your home network.",
      trustStore: "Your files and their index stay on your PC; no usage data is collected. InstaFind goes online only to fetch exchange rates, and its updates come from the Microsoft Store.",
      tips: [["report .pdf", "PDF files only"], ["size:>10mb", "Bigger than 10 MB"], ["dm:thisweek", "Changed this week"], ['"lease agreement"', "Exact phrase"]],
      // (Shown only in a language written in Arabic script: see ARAB_SCRIPT.)
      tipsArab: [["root: كتاب", "Arabic root: مكتبة، الكاتب"], ["احمد", "Also finds «أحمد»"]],
      // (Latin, Cyrillic and Japanese only, FOLD_SCRIPT: a word of the language
      // typed without its accents, ё or katakana, that finds it written with them.)
      tipFold: ["cafe", "Also finds “café”"],
      kNav: "Move", kOpen: "Open", kFolder: "Folder", kMenu: "More",
      open: "Open", openFolder: "Open folder", copyPath: "Copy path", more: "More actions",
      fDetails: "File details", kLabel: "Keyboard shortcuts", fType: "Type", fSize: "Size", fMod: "Modified", fMatches: "Matches", folder: "Folder", file: "File", fileOf: (x) => `${x} file`,
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
      rateText: "Is InstaFind useful to you? A rating in the Store helps others find it.", rateOk: "Rate InstaFind in the Store", rateLater: "Not now", rateRegion: "Rate in the Store",
      mSwitch: "Open its folder in the Save/Open dialog", qsCancel: "Cancel the move", qsNone: "No result to move the dialog to. Change the search, or press Esc to cancel.", mOpenWith: "Open with…", mProps: "Properties",
      renameH: "Rename", renameLabel: "New name", renameOk: "Rename", cancel: "Cancel", close: "Close",
      recycleH: "Move to the Recycle Bin", recycleQ: ["Move “", "” to the Recycle Bin?"], recycleQDir: ["Move the folder “", "” and everything in it to the Recycle Bin?"],
      recycleSub: "You can restore it from the Recycle Bin.", recycleOk: "Move to Recycle Bin",
      matchesH: (name) => `All matches — ${name}`, line: (n) => `Line ${num(n)}`, firstLines: (n) => `(first ${num(n)} lines)`,
      matchesNone: "Every line that matches the search shows here.",
      gone: "Moved to the Recycle Bin", offline: "drive not connected",
      history: "History", historyKey: "History (Ctrl+H)", savedH: "Saved searches", recentH: "Recent searches", recentEmpty: "No recent searches yet",
      saveCurrent: "Save the current search", clearRecent: "Clear history and recent files", clearSaved: "Delete all saved searches",
      savedInside: "inside documents", savedScoped: "in a folder",
      settings: "Settings", settingsKey: "Settings (Ctrl+,)", back: "Back to search",
      sLang: "Language", sLangNote: "The interface changes language after the program restarts.", restart: "Restart now",
      sLangMachine: "(machine translation)", sLangHelp: "Help improve this translation",
      sDisplay: "Display", sDigits: "Show Arabic-Indic digits (٠١٢٣)",
      app: "App", appsLabel: "Programs by this name",
      grid: "Thumbnail view (Ctrl+T)",
      clearQ: "Clear search", filtersBtn: "Filters", clearFilters: "Clear all filters", ixContents: (n) => `Indexing document contents · ${n}%`,
      theme: "Theme", themeSub: "“Automatic” follows Windows' light or dark mode", themes: { auto: "Automatic", light: "Light", dark: "Dark", midnight: "Midnight blue", kiwi: "Kiwi" },
      compact: "Compact rows", compactSub: "One line per result, so more of them fit", showPreview: "Show the preview", hidePreview: "Hide the preview",
      sMore: "More", sMoreRow: "More options", sMoreSub: "Check for updates, support and feedback, tools such as duplicate files and folder sizes, and search-from-anywhere settings", moreBtn: "Open the Options menu…",
      sHelp: "Updates and help", sUpdRow: "Updates", sUpdSub: (v) => `Installed version ${v}. Looks for a newer version and offers to install it.`, updBtn: "Check for updates…", updChecking: "Checking for updates…",
      sUpdSubStore: (v) => `Installed version ${v}. Updates come from the Microsoft Store.`, updBtnStore: "Updates from Microsoft Store…",
      sReportRow: "Report a problem", sReportSub: "Shows what would be shared first; then you send it by email or on the GitHub page (which is public: anyone can read it). Nothing is sent automatically, and file names and paths are removed.", reportBtn: "Report a problem…",
      sSearch: "Search and indexing", sHideSystem: "Hide system files", sHideSystemSub: "The Windows folder, temporary and cache files",
      sContentIndex: "Index document contents in the background", sContentIndexSub: "Faster search inside files, text in pictures and scans included",
      sReadCloud: "Read contents of cloud-only files (downloads them)",
      sReadPrograms: "Also read inside program and system files",
      sAutoIndex: "Update the index automatically when files change", reindex: "Update the index now",
      sAnywhere: "Search from anywhere", sHotkey: (k) => `${k} opens the small search box; when off, it opens this window.`,
      sHotkeyKeys: "Shortcut to open the search", sHotkeyKeysSub: "Works in any program. Pick another if another program uses it.",
      sHotkeyTaken: (k) => `Another program has ${k}, so it does nothing now. Pick another shortcut:`,
      sFloating: "Search box above the clock", sTray: "Icon in the notification area",
      sStartup: "Startup", sAutostart: "Start with Windows (in the background)",
      sPhone: "Search from phone", sPhoneRow: "Search from phone (home network)", sPhoneSub: "Search this PC's files from your phone's browser on the same Wi-Fi. Read-only, and only on your home network.",
      sPhoneOpen: "Open in the phone's browser:", sPhonePin: "and type the PIN:", sPhoneQr: "Or scan this code with the phone's camera to open the search without the PIN. Don't show it to anyone.",
      sPhoneQrLabel: "QR code that opens phone search", sPhoneOff: "Off. Turn it on to see the address and the PIN.", sPhoneWait: "Checking the network…",
      sPhoneIdle: "It turned itself off after two hours with no request from a phone, to be safe. Turn it on again when you need it.", sPhoneAllow: "Allow this network", sPhoneMore: "More settings…", sPhoneMoreSub: "The port, downloading files, and a new PIN and link",
      sAbout: "About", version: (v) => `InstaFind — version ${v}`,
      helpH: "How to search", helpKey: "How to search (F1)", helpMore: "All the ways to search…", helpSub: "Click an example to put it in the search box.", helpSubArab: "Every filter also has an Arabic spelling.",
      helpEx: [["sales report", "names with both words"], ['"lease agreement"', "the exact phrase"], ["report .pdf", "PDF files only (or ext:pdf)"], ["ext:docx,xlsx", "several types"],
        ["photo|video", "either word"], ["contract -old", "without “old” (or !old)"], ["size:>10mb", "bigger than 10 MB"], ["dm:thisweek", "changed this week"],
        ["dm:2026-05", "changed in May 2026"], ["ww:contract", "whole word only"], ['"lease contract"~5', "both words within 5 words"], ["folder: photos", "folders only"],
        ["fuzzy: reprot", "tolerates a typo"]],
      helpExArab: [["root: كتاب", "same Arabic root: مكتبة، الكاتب"], ["حجم:>1ج", "in Arabic too: bigger than 1 GB"], ["احمد", "also finds «أحمد»"]],
      helpInside: "To search inside documents (PDF, Word, Excel, pictures): press “Inside documents”, then Enter.",
      helpKeysH: "Keyboard",
      helpKeys: (hk) => [[hk, "the small search box from any program"], ["Ctrl+Alt+G", "in a Save/Open dialog: go to a result's folder"], ["Enter", "open the file"], ["Ctrl+Enter", "open its folder"], ["↓", "to the results (in an empty box: history)"],
        ["Ctrl+C", "copy the file"], ["Ctrl+Shift+C", "copy the path"], ["F2", "rename"], ["Delete", "to the Recycle Bin"], ["F4", "all matches in the file"], ["Ctrl+H", "history"], ["Ctrl+T", "thumbnails"], ["Ctrl+,", "settings"], ["F1", "this card"]],
    },
  });
  const SORTS = ["best", "name", "path", "size", "date"];
  // The examples ("Try these", the help card) that make sense in the
  // interface's script (src/i18n.rs LOCALES, ISO 15924): the Arabic ones
  // (roots, hamza and alef spellings, Arabic filters) only in a language
  // written in Arabic script; the accent fold (cafe → café, елка → ёлка,
  // かめら → カメラ) in a Latin, Cyrillic or Japanese one; the rest everywhere.
  const SCRIPT = typeof I18N.script === "string" ? I18N.script : LANG === "ar" ? "Arab" : "Latn";
  const ARAB_SCRIPT = SCRIPT === "Arab", FOLD_SCRIPT = ["Latn", "Cyrl", "Jpan"].includes(SCRIPT);
  const tipList = () => ARAB_SCRIPT ? [...T.tips, ...T.tipsArab] : FOLD_SCRIPT ? [...T.tips, T.tipFold] : T.tips;
  const helpExList = () => ARAB_SCRIPT ? [...T.helpEx, ...T.helpExArab] : FOLD_SCRIPT ? [...T.helpEx, T.tipFold] : T.helpEx;
  // The privacy line: the Store version's updates come from the Store, not GitHub.
  function trustText() { return S.store ? T.trustStore : T.trust; }

  // ---------- state ----------
  const S = {
    seq: 0, shown: -1, ver: 0, recentVer: 0, q: "", kind: 0, size: 0, date: 0, content: false, sort: "best", desc: false, scope: "",
    kinds: [], sizes: [], dates: [], digits: false, version: "",
    rows: [], total: 0, ms: 0, done: true, sel: 0, asked: 0, running: false, isContent: false, contentFor: null,
    progress: "", warnings: [], files: 0, state: "ready", stateText: "",
    recent: [], sugg: [], ssel: 0, view: "empty", note: "", notice: "", resultsScope: "",
    matches: null, history: { recent: [], saved: [] }, settings: null, apps: { seq: -1, apps: [] }, rsel: 0,
    // Ctrl+Alt+G: a Save/Open dialog waits for a result's folder (its hint), "" when none.
    qswitch: "",
  };

  const $ = (id) => document.getElementById(id);
  const q = $("q"), list = $("list");

  // ---------- numbers, sizes, dates ----------
  // The "Arabic-Indic digits" option: ٠١٢٣ in Arabic and Kurdish, the
  // Persian forms ۰۱۲۳ in Persian, Urdu and Pashto; Western digits in
  // every other language.
  const AR_DIGITS = "٠١٢٣٤٥٦٧٨٩", FA_DIGITS = "۰۱۲۳۴۵۶۷۸۹";
  const NATIVE_DIGITS = { ar: AR_DIGITS, ckb: AR_DIGITS, fa: FA_DIGITS, ur: FA_DIGITS, ps: FA_DIGITS }[CODE];
  function digits(s) { return S.digits && NATIVE_DIGITS ? String(s).replace(/[0-9]/g, (d) => NATIVE_DIGITS[d]) : String(s); }
  function num(n) { return digits(Number(n).toLocaleString("en-US")); }
  function size(bytes) {
    let v = Number(bytes), u = 0;
    while (v >= 1024 && u < T.units.length - 1) { v /= 1024; u++; }
    const s = u === 0 ? String(v) : v >= 10 ? v.toFixed(0) : v.toFixed(1).replace(/\.0$/, "");
    return `${digits(s)} ${T.units[u]}`;
  }
  // Dates in the interface language when the browser knows it (Western
  // digits; digits() converts them), else in English.
  const LOCALE = CODE === "en" ? "en-US" : (() => {
    try { return Intl.DateTimeFormat.supportedLocalesOf([CODE]).length ? `${CODE}-u-nu-latn` : "en-US"; } catch (e) { return "en-US"; }
  })();
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
    info: [["circle", { cx: "12", cy: "12", r: "9" }], ["path", { d: "M12 11v6M12 7.5v.5" }]],
    drive: [["rect", { x: "3", y: "8", width: "18", height: "9", rx: "2" }], ["path", { d: "M3 13h18M7 15.2h.01" }]],
    help: [["circle", { cx: "12", cy: "12", r: "9" }], ["path", { d: "M9.6 9.3a2.5 2.5 0 1 1 3.4 2.4c-.6.3-1 .8-1 1.5v.4M12 16.8v.2" }]],
  };

  // File-type badge: colour by family, the extension as its label.
  const FAMILY = [
    ["#D93F3F", ["pdf"]], ["#1E8E4E", ["xls", "xlsx", "xlsm", "csv", "ods"]], ["#2F64D6", ["doc", "docx", "odt", "rtf"]],
    ["#D9682B", ["ppt", "pptx", "odp"]], ["#9A7A12", ["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "iso"]],
    ["#8A55C9", ["jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "tif", "tiff", "svg"]],
    ["#C2417F", ["mp3", "wav", "m4a", "aac", "flac", "ogg", "wma", "opus", "mp4", "mkv", "avi", "mov", "wmv", "webm"]],
    ["#123B2A", ["exe", "msi", "bat", "cmd", "ps1", "lnk", "appx", "msix"]],
    // Code and markup: yellow, its label dark (white on yellow is unreadable).
    ["#E3B341", ["js", "mjs", "cjs", "jsx", "ts", "tsx", "json", "py", "rs", "go", "java", "c", "cpp", "h", "cs", "php", "rb", "sh", "html", "htm", "css", "xml", "yml", "yaml", "toml", "sql"], "#3A2E05"],
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
      for (const [k, v] of Object.entries({ x: "14.5", y: "25", "text-anchor": "middle", "font-size": label.length > 3 ? "6.2" : "7.5", "font-weight": "600", fill: (fam && fam[2]) || "#fff" })) t.setAttribute(k, v);
      t.textContent = label;
      svg.append(t);
    }
    return svg;
  }

  // Phone numbers, dates, versions, times and ranges («0770-123-4567»,
  // «2026-11-12», «12/11/2026», «2.0.1», «10:30», «2024–2026»): groups of
  // digits joined by one - / . : , or one of the hyphens and dashes U+2010–2013,
  // the minus sign U+2212 or the division slash U+2215 (the runs of
  // src/bidi_layout.rs `digit_runs`, the same joiners). After an Arabic letter the
  // bidi algorithm takes their digits for Arabic numbers, which "-" does not
  // join, so a right-to-left line showed "4567-123-0770" and "12-11-2026".
  // Each run goes in a left-to-right isolate that also overrides the
  // direction of its characters (.run in app.css): an isolate alone keeps
  // "٢٠٢٦-١١-١٢" reversed, ٠–٩ being Arabic numbers in any context. A run is
  // only digits and separators, so the override changes nothing else. The
  // text itself is unchanged.
  const DIGIT_RUN = /[0-9\u0660-\u0669\u06F0-\u06F9]+(?:[-/.:,\u2010-\u2013\u2212\u2215][0-9\u0660-\u0669\u06F0-\u06F9]+)+/g;
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
      S.digits = !!m.digits; S.version = m.version || ""; S.store = !!m.store;
      for (const el of document.querySelectorAll('[data-i18n="trust"]')) el.textContent = trustText();
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
      list.setAttribute("aria-busy", String(S.running));
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
    history(m) { S.history = { recent: m.recent || [], saved: m.saved || [], clearable: !!m.clearable }; if (!$("hist").hidden) renderHistory(); },
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
      if (m.theme && m.theme !== curTheme()) applyTheme(m.theme);
      $("hk-kbd").textContent = hotkeyText();
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
    // The search of the page before, after the program closed WebView2 while
    // the window waited hidden (to free its memory): text and filters back.
    restore(m) {
      closeSettings(); q.value = m.q;
      S.kind = m.kind < S.kinds.length ? m.kind : 0;
      S.size = m.size < S.sizes.length ? m.size : 0;
      S.date = m.date < S.dates.length ? m.date : 0;
      S.content = !!m.content;
      renderChips(); updatePlaceholder();
      runSearch();
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
    // The Store build's one-time rating question (the program decides when):
    // a row above the status line, never a dialog, never taking the focus.
    rating() {
      const bar = $("rating");
      const answer = (rate) => { bar.hidden = true; bar.replaceChildren(); send({ op: "rating", rate }); };
      bar.replaceChildren(h("span", null, T.rateText),
        btn("primary small", I.star, T.rateOk, () => answer(true), { id: "rate-ok" }),
        btn("small", I.x, T.rateLater, () => answer(false), { id: "rate-later" }));
      bar.hidden = false;
    },
    quick_switch(m) {
      S.qswitch = m.on ? m.text || "" : "";
      renderQswitch();
      // No suggestion is Enter's default while a dialog waits.
      if (S.view === "nores") renderNoResults();
      if (m.on) { closeSettings(); $("live").textContent = m.text || ""; }
    },
    notice(m) {
      S.notice = m.text; S.noticeWarn = !!m.warn; renderStatus();
      clearTimeout(handlers.noticeTimer);
      // Read out ("Path copied.", "Could not…"); a problem stays until the
      // next key or click.
      $("live").textContent = m.text;
      if (!m.warn) handlers.noticeTimer = setTimeout(() => { S.notice = ""; renderStatus(); }, 3500);
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
  // (Two-line rows unless the window is very low; the preview pane by
  // itself only where the list keeps its room, as in Explorer.)
  function compactOn() { return typeof PREF.compact === "boolean" ? PREF.compact : innerHeight < 600; }
  function previewOn() { return typeof PREF.preview === "boolean" ? PREF.preview : innerWidth >= 1200; }
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
  // Every column can be sized (kept in this page's storage, as the classic
  // list keeps its columns): the name and path column takes what is left
  // until it is given a width of its own (null).
  const COLW = { name: null, size: 96, date: 128 };
  const COL_MIN = { name: 160, size: 64, date: 64 }, COL_MAX = { name: 4000, size: 320, date: 320 };
  function colWidth(key) { const w = PREF.cols && PREF.cols[key]; return typeof w === "number" ? w : COLW[key]; }
  function setColWidth(key, w, save = true) {
    PREF.cols = Object.assign({}, PREF.cols, { [key]: w === null ? null : Math.round(Math.max(COL_MIN[key], Math.min(COL_MAX[key], w))) });
    applyCols();
    if (save) savePref();
  }
  function applyCols() {
    document.body.style.setProperty("--w-size", colWidth("size") + "px");
    document.body.style.setProperty("--w-date", colWidth("date") + "px");
    // (Never wider than the list: it gives way when the window narrows.)
    const name = colWidth("name");
    if (name === null) document.body.style.removeProperty("--c-name");
    else document.body.style.setProperty("--c-name", `minmax(0, ${name}px)`);
    for (const g of document.querySelectorAll(".grip")) g.setAttribute("aria-valuenow", String(Math.round(g.parentElement.getBoundingClientRect().width)));
  }
  function setPref(key, on) { PREF[key] = on; savePref(); applyView(); renderChips(); if (S.view === "settings") renderSettings(); }
  function viewButtons() {
    const b = (id, ic, on, label, click) => {
      const el = btn("icon view", ic, null, click, { id, role: "switch", "aria-checked": String(on), "aria-label": label, title: label });
      return el;
    };
    return h("span", { class: "viewbtns" },
      b("view-grid", I.grid, gridOn(), T.grid, () => toggleGrid()),
      b("view-compact", I.rows, compactOn(), T.compact, () => setPref("compact", !compactOn())),
      b("view-preview", I.pane, previewOn(), T.showPreview, () => setPref("preview", !previewOn())));
  }
  window.addEventListener("resize", () => { if (typeof PREF.compact !== "boolean" || typeof PREF.preview !== "boolean") { applyView(); renderChips(); } });
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
  // A subtle note on a result whose drive is not connected (its entry is kept).
  function offNote(r) {
    return r.offline ? h("span", { class: "off" }, T.offline) : null;
  }
  function tileEl(r, i) {
    const el = h("div", Object.assign({ class: "row tile" + (r.gone ? " gone" : "") }, rowAria(r, i, "results")));
    el.dataset.i = i;
    el.append(h("span", { class: "tpic" }, badge(r, "ft big"), h("img", { alt: "", draggable: "false" })), h("span", { class: "tname" }, nameEl(r)));
    if (r.offline) el.append(offNote(r));
    const key = thumbKeys.get(`${S.shown}:${S.ver}:${i}`);
    if (key) showThumb(el, key);
    else if (!r.gone && !r.offline) thumbSeen.observe(el);
    return el;
  }
  function toggleGrid() {
    PREF.grid = !gridOn(); savePref(); applyView(); renderChips();
    if (S.view === "results") { renderCols(); renderRows(true); }
  }

  // What Narrator, NVDA and UI Automation get for a row: an option of its
  // listbox ("Results" / the recent files), selected or not, named by the
  // file, its folder, size, date and matches, with its place in the list
  // (the whole count, or -1 while a document search still finds more).
  function rowAria(r, i, listName) {
    const name = plainName(r), results = listName === "results";
    const label = [name, r.dir, r.folder ? T.folder : size(r.size), when(r.mtime), r.matches ? T.nMatches(r.matches) : "", r.gone ? T.gone : "", r.offline ? T.offline : ""].filter(Boolean).join(", ");
    return {
      role: "option", id: (results ? "r" : "rr") + i, "aria-label": label,
      "aria-selected": String(results ? i === S.sel : i === S.rsel),
      "aria-posinset": String(i + 1), "aria-setsize": String(results ? (S.running ? -1 : Math.max(S.total, S.rows.length)) : S.recent.length),
    };
  }
  function rowEl(r, i, listName) {
    if (listName === "results" && gridOn()) return tileEl(r, i);
    const el = h("div", Object.assign({ class: "row" + (r.gone ? " gone" : "") + (r.snippet ? " doc" : "") }, rowAria(r, i, listName)));
    el.dataset.i = i;
    const nm = h("span", { class: "nm" }, h("span", { class: "n1" }, nameEl(r), r.matches > 1 ? h("span", { class: "cnt num" }, T.nMatches(r.matches)) : null), h("span", { class: "n2" }, pathEl(r.dir), offNote(r)));
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
      // Its name says what a press does and how the list is ordered now.
      const name = `${T.sortBy}: ${label}${on ? ` (${S.desc ? T.desc : T.asc})` : ""}`;
      const b = h("button", { class: "colbtn" + (on ? " sorted" : "") + (cls ? " " + cls : ""), type: "button", "aria-pressed": String(on), "aria-label": name }, h("span", null, label));
      if (on) b.append(icon(S.desc ? I.down : I.up, "i caret"));
      b.dataset.sort = key;
      return b;
    };
    // Every column is made wider or narrower by dragging the edge at the
    // end of its header, as in the classic list (kept for next time), or
    // with the arrow keys on that edge; a double click gives the column its
    // usual width back (the name column: what is left).
    const label = { name: T.cName, size: T.cSize, date: T.cDate };
    const grip = (key) => {
      const g = h("span", { class: "grip", role: "separator", tabindex: "0", "aria-orientation": "vertical", "aria-label": T.resize(label[key]), "aria-valuemin": String(COL_MIN[key]), "aria-valuemax": String(COL_MAX[key]) });
      g.dataset.col = key;
      const rtl = () => document.documentElement.dir === "rtl";
      // The width now (the name column has none of its own until sized).
      const now = () => colWidth(key) ?? Math.round(g.parentElement.getBoundingClientRect().width);
      g.addEventListener("pointerdown", (e) => {
        e.preventDefault(); e.stopPropagation();
        const start = e.clientX, w0 = now();
        g.setPointerCapture(e.pointerId); g.classList.add("on");
        // The end edge follows the pointer: to the right grows a column in a
        // left-to-right page, to the left in a right-to-left one.
        const move = (ev) => setColWidth(key, w0 + (rtl() ? start - ev.clientX : ev.clientX - start), false);
        const up = () => { g.classList.remove("on"); g.removeEventListener("pointermove", move); savePref(); applyCols(); };
        g.addEventListener("pointermove", move);
        g.addEventListener("pointerup", up, { once: true });
      });
      g.addEventListener("keydown", (e) => {
        if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
        const grow = (e.key === "ArrowRight") !== rtl();
        setColWidth(key, now() + (grow ? 16 : -16));
        e.preventDefault(); e.stopPropagation();
      });
      g.addEventListener("dblclick", (e) => { setColWidth(key, COLW[key]); e.stopPropagation(); });
      g.addEventListener("click", (e) => e.stopPropagation());
      return g;
    };
    const size = col("size", T.cSize, "end"), date = col("date", T.cDate, "end");
    $("cols").replaceChildren(h("span"), h("span", { class: "cwrap start" }, col("name", T.cName), grip("name")), h("span", { class: "cwrap end" }, size, grip("size")), h("span", { class: "cwrap end" }, date, grip("date")));
    applyCols();
  }
  function renderRows(refresh) {
    const top = list.scrollTop;
    list.setAttribute("aria-busy", String(!!S.running));
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
    // A named list, each label with its value in one item (what a screen reader reads as a row).
    const fact = (k, v, attrs) => h("div", { class: "fact", role: "listitem" }, h("span", { class: "k" }, k), h("span", Object.assign({ class: "v" }, attrs), v));
    const facts = h("div", { class: "facts", role: "list", "aria-label": T.fDetails }, fact(T.fType, kind));
    if (!r.folder) facts.append(fact(T.fSize, size(r.size), { class: "v num" }));
    facts.append(fact(T.fMod, fullDate(r.mtime), { class: "v num", title: fullDate(r.mtime) }));
    if (r.matches) facts.append(fact(T.fMatches, T.nMatches(r.matches), { class: "v num" }));
    const open = btn("primary", I.open, T.open + " ", () => act("open", S.rows[S.sel]), { disabled: r.gone });
    open.append(h("kbd", null, "Enter"));
    const parts = [
      h("div", { class: "thumb" }, badge(r)),
      h("div", null, h("div", { class: "pv-name", dir: firstStrongDir(plainName(r)) }, nameEl(r)), h("div", { class: "pv-path" }, pathEl(r.dir, 999))),
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
    // (Not a tab stop while it lists nothing.)
    el.tabIndex = S.recent.length ? 0 : -1;
    selectRecent(Math.min(S.rsel || 0, Math.max(0, S.recent.length - 1)));
  }
  // The recent files from the keyboard too (Tab to them, then the arrows):
  // the selected one is what Enter, Ctrl+Enter, F2, Delete and the menu act on.
  function selectRecent(i, scroll = false) {
    const el = $("recent");
    S.rsel = i;
    for (const r of el.querySelectorAll('.row[aria-selected="true"]')) r.setAttribute("aria-selected", "false");
    const row = $("rr" + i);
    if (!row) { el.removeAttribute("aria-activedescendant"); return; }
    row.setAttribute("aria-selected", "true");
    el.setAttribute("aria-activedescendant", row.id);
    if (scroll) row.scrollIntoView({ block: "nearest" });
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
    // While a Save/Open dialog waits (Ctrl+Alt+G), Enter is for moving it:
    // no suggestion is its default then (one searches all drives); they
    // can still be clicked.
    const pick = S.qswitch ? -1 : S.ssel;
    $("sugg").replaceChildren(...S.sugg.map((s, k) => {
      const el = h("div", { class: "sg", role: "option", id: "s" + k, "aria-selected": String(k === pick) },
        h("span", { class: "ic" }, icon(SUGG_ICON[s.kind] || I.search)),
        h("span", { class: "tx" }, h("b", null, s.label), h("span", null, T.sugg[s.kind] || "")),
        k === pick ? h("kbd", null, "Enter") : null);
      el.dataset.k = k;
      return el;
    }));
    if (pick >= 0 && S.sugg.length) q.setAttribute("aria-activedescendant", "s" + pick); else q.removeAttribute("aria-activedescendant");
  }
  function applySuggestion(s) {
    if (!s) return;
    if (s.kind === "query") { q.value = s.q; runSearch(); }
    else if (s.kind === "all_kinds") setFilter("kind", 0);
    else if (s.kind === "all_drives") send({ op: "all_drives" });
    else if (s.kind === "contents") { S.content = true; renderChips(); runSearch(); }
    else if (s.kind === "web") send({ op: "web", q: S.q });
  }
  // The index at the status bar's end: while indexing, how far it got with
  // a thin bar (a percentage while document contents are read: "checked of
  // total" in the program's words, in either order); else its size.
  function renderBanner() {
    const busy = S.state !== "ready", bar = $("ix-bar");
    // (The tooltip keeps the program's own words, e.g. "your folders first",
    // which the short percentage below leaves out.)
    $("ix").title = busy ? (S.stateText ? digits(S.stateText) + "\n" : "") + T.ixSub : "";
    const say = (text) => { $("ix-text").replaceChildren(); fill($("ix-text"), [[text, false]]); };
    if (!busy) { say(S.files ? T.files(S.files) : ""); bar.hidden = true; return; }
    let pct = null;
    const n = S.state === "contents" ? (S.stateText || "").match(/\d[\d,]*/g) : null;
    if (n && n.length >= 2) {
      const [a, b] = n.slice(-2).map((x) => Number(x.replace(/,/g, "")));
      if (Math.max(a, b) > 0) pct = Math.floor((Math.min(a, b) * 100) / Math.max(a, b));
    }
    say(S.state === "loading" ? T.loading : pct !== null ? T.ixContents(num(pct)) : digits(S.stateText));
    bar.hidden = false;
    bar.classList.toggle("busy", pct === null);
    bar.firstElementChild.style.width = pct === null ? "" : pct + "%";
  }
  function renderStatus() {
    let text = "";
    if (S.notice) text = S.notice;
    else if (S.view !== "empty" && S.view !== "settings") {
      if (S.content && contentStale()) text = T.pressEnter;
      else if (S.isContent && S.progress) text = digits(S.progress);
      else text = (S.note ? S.note + " · " : "") + T.count(S.total, S.ms);
      if (S.warnings.length) text += ` — ${T.note}: ${S.warnings.join(["ar", "fa", "ur", "ps", "ckb", "ug"].includes(CODE) ? "، " : "; ")}`;
    } else if (S.content && contentStale() && S.q.trim()) text = T.pressEnter;
    $("status-text").replaceChildren();
    fill($("status-text"), [[text, false]]);
    $("status-text").classList.toggle("warn", !!(S.notice && S.noticeWarn));
    renderBanner();
  }
  for (const ev of ["keydown", "mousedown"]) document.addEventListener(ev, () => { if (S.notice && S.noticeWarn) { S.notice = ""; S.noticeWarn = false; renderStatus(); } }, true);
  function renderStop() { $("stop").hidden = !S.running; }
  // The result count for screen readers, once typing pauses.
  let liveTimer = 0;
  function announce() {
    clearTimeout(liveTimer);
    liveTimer = setTimeout(() => { $("live").textContent = T.nres(S.total); }, 500);
  }

  // ---------- the toolbar: Filters, the scope, inside documents, the view, the order ----------
  // A button with its menu of choices ([label, chosen, data]); `pick(item)`
  // gets the clicked item's element.
  function menuChip(id, ic, label, attrs, items, pick, end) {
    const chip = h("button", Object.assign({ class: "chip" + (end ? " end" : ""), id: "chip-" + id, type: "button", "aria-haspopup": "menu", "aria-expanded": "false" }, attrs), icon(ic));
    chip.append(...[].concat(label), icon(I.down, "i caret"));
    const menu = h("div", { class: "menu", role: "menu", id: "menu-" + id, "aria-label": attrs["aria-label"] || T.filtersBtn, hidden: true }, ...items);
    const wrap = h("span", { class: "chipwrap" + (end ? " end" : "") }, chip, menu);
    chip.addEventListener("click", (e) => { if (e.target.closest("[data-clear]")) { send({ op: "all_drives" }); return; } toggleMenu("menu-" + id); });
    menu.addEventListener("click", (e) => { const it = e.target.closest("[data-k]"); if (it) { closeMenus(); pick(it); } });
    return wrap;
  }
  const radio = (label, on, data) => { const it = h("div", { role: "menuitemradio", tabindex: "-1", "aria-checked": String(on) }, label); Object.assign(it.dataset, data); return it; };
  const tsep = () => h("span", { class: "tsep", "aria-hidden": "true" });
  function renderChips() {
    // Filters: type, size and date in one menu, a group each.
    const groups = [["kind", T.type, S.kinds, S.kind], ["size", T.size, S.sizes, S.size], ["date", T.date, S.dates, S.date]];
    const on = groups.filter(([, , items, cur]) => cur > 0 && items[cur]);
    const fItems = groups.map(([g, title, items, cur]) => h("div", { class: "mgroup", role: "group", "aria-label": title }, h("div", { class: "mh", role: "presentation" }, title), ...items.map((label, i) => radio(label, i === cur, { g, k: i }))));
    if (on.length) {
      const clear = h("div", { role: "menuitem", tabindex: "-1", id: "filters-clear" }, icon(I.x), h("span", null, T.clearFilters));
      clear.dataset.k = "-1";
      fItems.push(h("div", { class: "sep", role: "separator" }), clear);
    }
    const fLabel = on.length ? `${T.filtersBtn}: ${on.map(([, t, items, cur]) => `${t}: ${items[cur]}`).join("، ")}` : T.filtersBtn;
    const filters = menuChip("filters", I.filter, [h("span", { class: "cl" }, T.filtersBtn), on.length ? h("span", { class: "fcount num" }, num(on.length)) : null].filter(Boolean),
      { "aria-label": fLabel, title: on.length ? fLabel : null }, fItems,
      (it) => { if (it.dataset.k === "-1") { S.kind = 0; S.size = 0; S.date = 0; S.kindLabel = ""; renderChips(); q.focus(); if (S.q.trim()) runSearch(); } else setFilter(it.dataset.g, +it.dataset.k); });
    filters.querySelector(".chip").classList.toggle("on", on.length > 0);
    filters.querySelector(".menu").classList.add("fmenu");
    // The scope: all drives, or a folder (its × goes back to all drives).
    const scopeLabel = S.scope ? [h("span", { class: "sr" }, `${T.lookIn}: `), pathEl(S.scope, 48)] : [h("span", { class: "cl" }, T.allDrives)];
    const scope = menuChip("scope", S.scope ? I.folder : I.drive, scopeLabel, { "aria-label": S.scope ? `${T.lookIn}: ${S.scope}` : `${T.lookIn}: ${T.allDrives}` },
      [T.allDrives, T.chooseFolder].map((label, i) => radio(label, i === (S.scope ? -1 : 0), { k: i })), (it) => send({ op: +it.dataset.k ? "pick_folder" : "all_drives" }));
    if (S.scope) {
      const chip = scope.querySelector(".chip"), caret = chip.querySelector(".caret");
      chip.classList.add("on");
      const x = icon(I.x, "i caret"); x.dataset.clear = "1"; x.setAttribute("aria-label", T.clearFilter);
      caret.replaceWith(x);
    }
    // Inside documents: a switch.
    const content = h("button", { class: "chip" + (S.content ? " on" : ""), id: "chip-content", type: "button", role: "switch", "aria-checked": String(S.content) }, icon(I.text), h("span", { class: "cl" }, T.inside));
    content.addEventListener("click", () => { S.content = !S.content; renderChips(); q.focus(); updatePlaceholder(); if (S.q.trim()) runSearch(); });
    // The order, at the far end.
    const sortLabel = `${T.sortBy}: ${T.sorts[S.sort]}${S.sort === "best" ? "" : ` (${S.desc ? T.desc : T.asc})`}`;
    const sort = menuChip("sort", I.sort, h("span", { class: "cl" }, T.sorts[S.sort]), { "aria-label": sortLabel, title: sortLabel },
      SORTS.map((k, i) => radio(T.sorts[k], i === SORTS.indexOf(S.sort), { k: i })), (it) => { const k = SORTS[+it.dataset.k]; setSort(k, k === S.sort ? !S.desc : false); }, true);
    $("chips").replaceChildren(filters, tsep(), scope, tsep(), h("span", { class: "chipwrap" }, content), h("span", { class: "grow" }), viewButtons(), tsep(), sort);
    updatePlaceholder();
  }
  // (The box's accessible name follows its hint: names or inside documents.)
  function updatePlaceholder() { q.placeholder = S.content ? T.phContent : T.ph; q.setAttribute("aria-label", q.placeholder); }
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
      const it = h("div", { role: "menuitem", tabindex: "-1", "data-op": op }, icon(ic), h("span", null, label), key ? h("kbd", null, key) : null);
      it.addEventListener("click", () => { closeMenus(); runCtx(op); });
      return it;
    };
    // While a Save/Open dialog waits (Ctrl+Alt+G), its move comes first and takes Enter.
    const items = S.qswitch ? [item(T.mSwitch, "Enter", "switch_to", I.folder), item(T.mOpen, null, "open", I.open)] : [item(T.mOpen, "Enter", "open", I.open)];
    items.push(item(T.mReveal, "Ctrl+Enter", "reveal", I.folder));
    // Windows' own "Open with" choice (files only), as the classic menu.
    if (!r.folder) items.push(item(T.mOpenWith, null, "open_with", I.open));
    items.push(item(T.mCopyFile, "Ctrl+C", "copy_file", I.copy), item(T.mCopyPath, "Ctrl+Shift+C", "copy", I.copy));
    if (listName === "results") items.push(item(T.mCopyAll, null, "copy_all", I.copy));
    items.push(item(T.mScope, null, "scope", I.search), item(T.mSizes, null, "folder_sizes", I.scale));
    if (listName === "results") items.push(item(T.mReplace, null, "replace_text", I.pen));
    if (r.snippet) items.push(item(T.mMatches, "F4", "matches", I.text));
    items.push(h("div", { class: "sep", role: "separator" }), item(T.mRename, "F2", "rename", I.pen), item(T.mRecycle, "Delete", "recycle", I.trash));
    // Last, as in Explorer: Windows' Properties window.
    items.push(h("div", { class: "sep", role: "separator" }), item(T.mProps, "Alt+Enter", "properties", I.info));
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
    $("dlg").className = "dlg";
    $("dlg-h").replaceChildren(...[].concat(title));
    $("dlg-body").replaceChildren(...body);
    $("dlg-btns").replaceChildren(...buttons);
    $("veil").hidden = false;
  }
  function closeDialog() {
    $("veil").hidden = true; dlgFor = "";
    (dlgBack && dlgBack.isConnected ? dlgBack : q).focus();
  }
  // «كيف أبحث؟» (the ? button, F1): examples that go in the box when
  // clicked, and the keys. Static text only.
  function helpCard() {
    closeMenus();
    const ex = h("div", { class: "hx-list", role: "list", "aria-label": T.helpH }, ...helpExList().map(([c, d], i) => {
      const b = h("button", { class: "hx", type: "button", id: "hx" + i }, h("code", { dir: "auto" }, c), h("span", null, d));
      b.addEventListener("click", () => { closeDialog(); closeSettings(); q.value = c; onInput(); q.focus(); });
      return h("div", { role: "listitem" }, b);
    }));
    const keys = h("dl", { class: "hk-list", "aria-labelledby": "hk-h" }, ...T.helpKeys(hotkeyText()).flatMap(([k, d]) => [h("dt", null, h("kbd", null, k)), h("dd", null, d)]));
    dialog(T.helpH, [h("p", { class: "sub" }, ARAB_SCRIPT && T.helpSubArab ? `${T.helpSub} ${T.helpSubArab}` : T.helpSub), ex, h("p", { class: "sub" }, T.helpInside), h("h4", { id: "hk-h" }, T.helpKeysH), keys],
      [btn("", I.x, T.close, closeDialog, { id: "help-close" })], "help");
    $("dlg").classList.add("help");
    $("hx0").focus();
  }
  // The global hotkey's keys (Settings can change them).
  function hotkeyText() {
    const st = S.settings, hk = st && (st.hotkeys || []).find(([code]) => code === st.hotkey);
    return hk ? hk[1] : "Ctrl+Alt+F";
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
    const item = (cls, ic, label, sub, onClick, disabled, id) => {
      const it = h("div", { role: "menuitem", tabindex: "-1", class: cls, id, "aria-disabled": disabled ? "true" : null }, icon(ic), h("bdi", { class: "lbl" }, label), sub ? h("span", { class: "sub" }, sub) : null);
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
    // Also the opened files and the quick search's launches, as the classic menu.
    kids.push(item("act", I.x, T.clearRecent, null, () => send({ op: "clear_recent" }), !S.history.clearable, "hist-clear"));
    kids.push(item("act", I.trash, T.clearSaved, null, () => send({ op: "clear_saved" }), !S.history.saved.length));
    menu.replaceChildren(...kids);
  }
  function openHistory() { renderHistory(); toggleMenu("hist"); if (!$("hist").hidden) $("hist").querySelector('[tabindex="-1"]:not([aria-disabled="true"])')?.focus(); }
  $("hist-btn").addEventListener("click", openHistory);

  // ---------- themes ----------
  // The page's colours (app.css, :root[data-theme]); "auto" follows
  // Windows' light or dark mode. Kept by the program (settings.txt); this
  // page's storage only paints the last one at once on the next start.
  const THEMES = ["auto", "light", "dark", "midnight", "kiwi"];
  function curTheme() { const t = document.documentElement.dataset.theme; return THEMES.includes(t) ? t : "auto"; }
  function applyTheme(t) {
    t = THEMES.includes(t) ? t : "auto";
    document.documentElement.dataset.theme = t;
    try { localStorage.setItem("instafind.theme", t); } catch { /* not kept */ }
  }
  function setTheme(t) {
    if (!THEMES.includes(t)) return;
    applyTheme(t);
    if (S.settings) S.settings.theme = t;
    send({ op: "theme", key: t });
    if (S.view === "settings") renderSettings();
  }
  try { applyTheme(localStorage.getItem("instafind.theme")); } catch { applyTheme("auto"); }

  // ---------- ⋮: the secondary actions ----------
  function renderMore() {
    const item = (id, ic, label, key, fn) => {
      const it = h("div", { role: "menuitem", tabindex: "-1", id }, icon(ic), h("span", null, label), key ? h("kbd", null, key) : null);
      it.addEventListener("click", () => { closeMenus(); fn(); });
      return it;
    };
    const sep = () => h("div", { class: "sep", role: "separator" });
    const themes = THEMES.map((t) => {
      const it = h("div", { role: "menuitemradio", tabindex: "-1", id: "more-theme-" + t, "aria-checked": String(t === curTheme()) }, h("span", { class: "swatch", "data-swatch": t, "aria-hidden": "true" }, h("i"), h("i")), h("span", null, T.themes[t]));
      it.addEventListener("click", () => { closeMenus(); setTheme(t); q.focus(); });
      return it;
    });
    const store = S.settings && S.settings.store;
    $("more-menu").replaceChildren(
      item("tool-export", I.export, T.exportBtn, null, () => send({ op: "export" })),
      item("tool-reindex", I.refresh, T.reindexBtn, null, () => send({ op: "reindex" })),
      item("more-help", I.help, T.helpH, "F1", helpCard),
      sep(), h("div", { class: "mh", role: "presentation" }, T.theme), h("div", { role: "group", "aria-label": T.theme }, ...themes),
      sep(), item("more-updates", I.refresh, store ? T.updBtnStore : T.updBtn, null, () => send({ op: "check_updates" })),
      item("more-report", I.flag, T.reportBtn, null, () => send({ op: "report_problem" })));
  }
  $("more-btn").addEventListener("click", () => { renderMore(); toggleMenu("more-menu"); });
  $("q-clear").addEventListener("mousedown", (e) => e.preventDefault());
  $("q-clear").addEventListener("click", () => { q.value = ""; onInput(); q.focus(); });

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
  // "Check for updates…": while a check runs it says so and waits (the
  // answer comes as a message box); it stays focusable, so the keyboard
  // and a screen reader keep their place.
  function updButton(st) {
    const busy = !st.store && !!st.checking_updates;
    const b = btn("", I.refresh, busy ? T.updChecking : st.store ? T.updBtnStore : T.updBtn, () => {
      if (S.settings.checking_updates) return;
      // Said at once; the program confirms (or not) on its next tick.
      if (!st.store) { S.settings.checking_updates = true; renderSettings(); }
      send({ op: "check_updates" });
    }, { id: "check-updates", "aria-describedby": "check-updates-d", "aria-disabled": busy ? "true" : null, "aria-busy": busy ? "true" : null });
    return b;
  }
  function renderSettings() {
    const st = S.settings;
    // (Drawn again when the program answers: the focus stays where it was.)
    const refocus = document.activeElement && document.activeElement.closest("#settings") && document.activeElement.id;
    const box = $("settings");
    if (!st) { box.replaceChildren(); return; }
    const sw = (key, label, sub) => {
      const on = !!st[key];
      const b = h("button", { class: "sw", type: "button", role: "switch", "aria-checked": String(on), id: "set-" + key, "aria-label": label }, h("span", { class: "knob" }));
      b.addEventListener("click", () => send({ op: "set", key, on: !on }));
      if (sub) b.setAttribute("aria-describedby", "set-" + key + "-d");
      return h("div", { class: "srow" }, h("label", { for: "set-" + key }, h("span", null, label), sub ? h("small", { id: "set-" + key + "-d" }, sub) : null), b);
    };
    const section = (title, ...rows) => h("section", { class: "sbox" }, h("h3", null, title), ...rows);
    // Every language by its own name, in its own script and font (app.css
    // :lang), a translation marked as a machine translation.
    const langs = Array.isArray(I18N.langs) && I18N.langs.length ? I18N.langs : [{ code: "ar", native: "العربية", rtl: true }, { code: "en", native: "English", rtl: false }];
    const langBtn = (l) => {
      const on = st.next_lang === l.code;
      const b = h("button", { class: "lopt" + (on ? " on" : ""), type: "button", role: "radio", "aria-checked": String(on), "data-lang": l.code, "aria-label": l.machine ? `${l.native} ${T.sLangMachine}` : l.native },
        h("span", { class: "lname", lang: l.code, dir: l.rtl ? "rtl" : "ltr" }, l.native),
        l.machine ? h("small", null, T.sLangMachine) : null);
      b.addEventListener("click", () => send({ op: "lang", lang: l.code }));
      return b;
    };
    const lang = section(T.sLang, h("div", { class: "langs", role: "radiogroup", "aria-label": T.sLang }, ...langs.map(langBtn)));
    if (langs.find((l) => l.code === CODE && l.machine)) {
      lang.append(h("div", { class: "srow" }, h("label", { for: "lang-help" }, h("span", null, T.sLangHelp)), btn("", I.flag, T.reportBtn, () => send({ op: "report_problem" }), { id: "lang-help" })));
    }
    if (st.next_lang !== st.lang) lang.append(h("div", { class: "snote" }, h("span", null, T.sLangNote), btn("primary", I.open, T.restart, () => send({ op: "restart" }))));
    const reindex = btn("", I.search, T.reindex, () => send({ op: "reindex" }));
    const back = btn("", I.back, T.back, closeSettings, { class: "btn back" });
    box.replaceChildren(
      h("div", { class: "shead" }, back, h("h2", null, T.settings)),
      lang,
      section(T.sDisplay, sw("digits", T.sDigits),
        (() => {
          const on = compactOn();
          const b = h("button", { class: "sw", type: "button", role: "switch", "aria-checked": String(on), id: "set-compact", "aria-label": T.compact, "aria-describedby": "set-compact-d" }, h("span", { class: "knob" }));
          b.addEventListener("click", () => setPref("compact", !compactOn()));
          return h("div", { class: "srow" }, h("label", { for: "set-compact" }, h("span", null, T.compact), h("small", { id: "set-compact-d" }, T.compactSub)), b);
        })(),
        themeRow()),
      section(T.sSearch, sw("hide_system", T.sHideSystem, T.sHideSystemSub), sw("content_index", T.sContentIndex, T.sContentIndexSub), sw("read_cloud_files", T.sReadCloud), sw("read_program_files", T.sReadPrograms), sw("auto_index", T.sAutoIndex), h("div", { class: "srow" }, reindex)),
      section(T.sAnywhere, sw("hotkey_quick", T.sHotkey(hotkeyText())), hotkeyRow(st), sw("floating", T.sFloating), sw("tray_icon", T.sTray)),
      ...[phoneSection(st, section)].filter(Boolean),
      section(T.sStartup, sw("autostart", T.sAutostart)),
      // "Check for updates" and "Report a problem", each with its own button
      // in view (the release standard), before the Options menu's rest.
      section(T.sHelp,
        // (A button is named by its own text; the card's words describe it.)
        h("div", { class: "srow" }, h("div", { class: "slbl" }, h("span", null, T.sUpdRow), h("small", { class: "num", id: "check-updates-d" }, digits((st.store ? T.sUpdSubStore : T.sUpdSub)(st.version)))),
          updButton(st)),
        h("div", { class: "srow" }, h("div", { class: "slbl" }, h("span", null, T.sReportRow), h("small", { id: "report-problem-d" }, T.sReportSub)), btn("", I.flag, T.reportBtn, () => send({ op: "report_problem" }), { id: "report-problem", "aria-describedby": "report-problem-d" }))),
      section(T.sMore, h("div", { class: "srow" }, h("div", { class: "slbl" }, h("span", null, T.sMoreRow), h("small", { id: "options-menu-d" }, T.sMoreSub)), btn("", I.dots, T.moreBtn, () => send({ op: "options_menu" }), { id: "options-menu", "aria-describedby": "options-menu-d" }))),
      section(T.sAbout, h("p", { class: "num" }, digits(T.version(st.version))), h("p", { class: "muted" }, trustText())),
    );
    if (refocus) $(refocus)?.focus({ preventScroll: true });
  }
  // The colour themes: a radio group (one tab stop, the arrows choose), each
  // with a small sample of its colours. A choice shows at once.
  function themeRow() {
    const cur = curTheme();
    const group = h("div", { class: "themes", role: "radiogroup", "aria-labelledby": "theme-label", "aria-describedby": "theme-d" }, ...THEMES.map((t) => {
      const b = h("button", { class: "thm" + (t === cur ? " on" : ""), type: "button", role: "radio", "aria-checked": String(t === cur), id: "theme-" + t, tabindex: t === cur ? "0" : "-1" },
        h("span", { class: "swatch", "data-swatch": t, "aria-hidden": "true" }, h("i"), h("i")), h("span", { class: "tn" }, T.themes[t]));
      b.addEventListener("click", () => { setTheme(t); $("theme-" + t)?.focus(); });
      return b;
    }));
    group.addEventListener("keydown", (e) => {
      const step = { ArrowDown: 1, ArrowUp: -1, ArrowRight: document.documentElement.dir === "rtl" ? -1 : 1, ArrowLeft: document.documentElement.dir === "rtl" ? 1 : -1 }[e.key];
      if (!step) return;
      const t = THEMES[(THEMES.indexOf(curTheme()) + step + THEMES.length) % THEMES.length];
      setTheme(t); $("theme-" + t)?.focus();
      e.preventDefault(); e.stopPropagation();
    });
    return h("div", { class: "srow themerow" }, h("div", { class: "slbl" }, h("span", { id: "theme-label" }, T.theme), h("small", { id: "theme-d" }, T.themeSub)), group);
  }
  // Search from phone (not in the Store edition, which has none): the
  // switch, then the address, the PIN and a QR code of the direct link.
  function phoneSection(st, section) {
    const ph = st.phone;
    if (!ph) return null;
    const b = h("button", { class: "sw", type: "button", role: "switch", "aria-checked": String(!!ph.on), id: "set-phone", "aria-label": T.sPhoneRow, "aria-describedby": "set-phone-d" }, h("span", { class: "knob" }));
    b.addEventListener("click", () => send({ op: "set", key: "phone", on: !ph.on }));
    const rows = [h("div", { class: "srow" }, h("label", { for: "set-phone" }, h("span", null, T.sPhoneRow), h("small", { id: "set-phone-d" }, T.sPhoneSub)), b)];
    // (It turns itself off after two hours with no request.)
    if (ph.running && ph.url) {
      const qr = qrSvg(ph.link || ph.url);
      rows.push(h("div", { class: "phone" }, qr,
        h("div", { class: "ptext" }, h("small", null, T.sPhoneOpen), h("bdi", { class: "purl", dir: "ltr", id: "phone-url" }, ph.url),
          h("small", null, T.sPhonePin), h("bdi", { class: "ppin", dir: "ltr", id: "phone-pin" }, ph.pin), qr ? h("small", null, T.sPhoneQr) : null)));
    } else if (ph.on) rows.push(h("p", { class: "pnote", id: "phone-note" }, ph.note || T.sPhoneWait));
    else rows.push(h("p", { class: ph.idle_off ? "pnote idle" : "muted", id: "phone-note" }, ph.idle_off ? T.sPhoneIdle : T.sPhoneOff));
    if (ph.running && ph.note) rows.push(h("p", { class: "pnote" }, ph.note));
    const btns = h("div", { class: "srow pbtns" });
    if (ph.on && !ph.running && ph.allow) btns.append(btn("primary", I.globe, T.sPhoneAllow, () => send({ op: "phone_allow" }), { id: "phone-allow" }));
    btns.append(h("small", { id: "phone-more-d" }, T.sPhoneMoreSub), btn("", I.dots, T.sPhoneMore, () => send({ op: "phone_window" }), { id: "phone-more", "aria-describedby": "phone-more-d" }));
    rows.push(btns);
    return section(T.sPhone, ...rows);
  }
  // The QR code of `text` as an SVG (dark on white whatever the theme, with
  // its quiet zone), or null when it does not fit.
  function qrSvg(text) {
    const m = qrMatrix(text);
    if (!m) return null;
    const n = m.length + 8, svg = document.createElementNS(SVG_NS, "svg");
    for (const [k, v] of Object.entries({ class: "qr", viewBox: `0 0 ${n} ${n}`, role: "img", "aria-label": T.sPhoneQrLabel, "shape-rendering": "crispEdges", id: "phone-qr" })) svg.setAttribute(k, v);
    const bg = document.createElementNS(SVG_NS, "rect");
    for (const [k, v] of Object.entries({ width: n, height: n, fill: "#fff" })) bg.setAttribute(k, v);
    let d = "";
    m.forEach((row, y) => row.forEach((dark, x) => { if (dark) d += `M${x + 4} ${y + 4}h1v1h-1z`; }));
    const p = document.createElementNS(SVG_NS, "path");
    p.setAttribute("d", d); p.setAttribute("fill", "#000");
    svg.append(bg, p);
    return svg;
  }

  // ---------- QR code (the phone-search link) ----------
  // A tiny QR encoder: byte mode, error correction M, versions 1–10 (up to
  // 213 bytes; the link is under 70), drawn on this PC with no network. The
  // module matrix follows ISO/IEC 18004 (as Nayuki's reference encoder);
  // null when the text does not fit.
  function qrMatrix(text) {
    const data = [...new TextEncoder().encode(text)];
    const ECC = [0, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26], BLOCKS = [0, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5];
    const rawModules = (v) => { let r = (16 * v + 128) * v + 64; if (v >= 2) { const n = Math.floor(v / 7) + 2; r -= (25 * n - 10) * n - 55; if (v >= 7) r -= 36; } return r; };
    let ver = 0, cap = 0;
    for (let v = 1; v <= 10 && !ver; v++) {
      cap = Math.floor(rawModules(v) / 8) - ECC[v] * BLOCKS[v];
      if (4 + (v < 10 ? 8 : 16) + data.length * 8 <= cap * 8) ver = v;
    }
    if (!ver) return null;
    // The bit stream: mode, count, bytes, terminator, padding.
    const bits = [];
    const put = (val, n) => { for (let i = n - 1; i >= 0; i--) bits.push((val >>> i) & 1); };
    put(4, 4); put(data.length, ver < 10 ? 8 : 16); for (const b of data) put(b, 8);
    put(0, Math.min(4, cap * 8 - bits.length)); put(0, (8 - bits.length % 8) % 8);
    const words = [];
    for (let i = 0; i < bits.length; i += 8) words.push(bits.slice(i, i + 8).reduce((a, b) => (a << 1) | b, 0));
    for (let pad = 0xEC; words.length < cap; pad ^= 0xEC ^ 0x11) words.push(pad);
    // Reed–Solomon over GF(256), blocks interleaved.
    const mul = (x, y) => { let z = 0; for (let i = 7; i >= 0; i--) { z = (z << 1) ^ ((z >>> 7) * 0x11D); z ^= ((y >>> i) & 1) * x; } return z & 0xFF; };
    const deg = ECC[ver], div = new Array(deg).fill(0); div[deg - 1] = 1;
    for (let i = 0, root = 1; i < deg; i++, root = mul(root, 2)) for (let j = 0; j < deg; j++) { div[j] = mul(div[j], root); if (j + 1 < deg) div[j] ^= div[j + 1]; }
    const rem = (block) => { const r = new Array(deg).fill(0); for (const b of block) { const f = b ^ r.shift(); r.push(0); div.forEach((c, i) => { r[i] ^= mul(c, f); }); } return r; };
    const nb = BLOCKS[ver], raw = Math.floor(rawModules(ver) / 8), short = nb - raw % nb, shortLen = Math.floor(raw / nb), blocks = [];
    for (let i = 0, k = 0; i < nb; i++) {
      const d = words.slice(k, k + shortLen - deg + (i < short ? 0 : 1)); k += d.length;
      const e = rem(d); if (i < short) d.push(0); blocks.push(d.concat(e));
    }
    const all = [];
    for (let i = 0; i < blocks[0].length; i++) for (let j = 0; j < nb; j++) if (i !== shortLen - deg || j >= short) all.push(blocks[j][i]);
    // The function patterns.
    const size = ver * 4 + 17, m = [], fn = [];
    for (let y = 0; y < size; y++) { m.push(new Array(size).fill(false)); fn.push(new Array(size).fill(false)); }
    const set = (x, y, dark) => { m[y][x] = dark; fn[y][x] = true; };
    for (let i = 0; i < size; i++) { set(6, i, i % 2 === 0); set(i, 6, i % 2 === 0); }
    for (const [cx, cy] of [[3, 3], [size - 4, 3], [3, size - 4]]) for (let dy = -4; dy <= 4; dy++) for (let dx = -4; dx <= 4; dx++) {
      const x = cx + dx, y = cy + dy, d = Math.max(Math.abs(dx), Math.abs(dy));
      if (x >= 0 && x < size && y >= 0 && y < size) set(x, y, d !== 2 && d !== 4);
    }
    if (ver > 1) {
      const n = Math.floor(ver / 7) + 2, step = Math.ceil((ver * 4 + 4) / (n * 2 - 2)) * 2, pos = [6];
      for (let p = size - 7; pos.length < n; p -= step) pos.splice(1, 0, p);
      for (let i = 0; i < n; i++) for (let j = 0; j < n; j++) {
        if ((i === 0 && j === 0) || (i === 0 && j === n - 1) || (i === n - 1 && j === 0)) continue;
        for (let dy = -2; dy <= 2; dy++) for (let dx = -2; dx <= 2; dx++) set(pos[i] + dx, pos[j] + dy, Math.max(Math.abs(dx), Math.abs(dy)) !== 1);
      }
    }
    const format = (mask) => {
      const d = mask; let r = d; // level M is 00
      for (let i = 0; i < 10; i++) r = (r << 1) ^ ((r >>> 9) * 0x537);
      const b = ((d << 10) | r) ^ 0x5412, bit = (i) => ((b >>> i) & 1) === 1;
      for (let i = 0; i <= 5; i++) set(8, i, bit(i));
      set(8, 7, bit(6)); set(8, 8, bit(7)); set(7, 8, bit(8));
      for (let i = 9; i < 15; i++) set(14 - i, 8, bit(i));
      for (let i = 0; i < 8; i++) set(size - 1 - i, 8, bit(i));
      for (let i = 8; i < 15; i++) set(8, size - 15 + i, bit(i));
      set(8, size - 8, true);
    };
    format(0);
    if (ver >= 7) {
      let r = ver; for (let i = 0; i < 12; i++) r = (r << 1) ^ ((r >>> 11) * 0x1F25);
      const b = (ver << 12) | r;
      for (let i = 0; i < 18; i++) { const dark = ((b >>> i) & 1) === 1, a = size - 11 + i % 3, c = Math.floor(i / 3); set(a, c, dark); set(c, a, dark); }
    }
    // The data, in the zigzag order.
    for (let right = size - 1, i = 0; right >= 1; right -= 2) {
      if (right === 6) right = 5;
      for (let vert = 0; vert < size; vert++) for (let j = 0; j < 2; j++) {
        const x = right - j, y = ((right + 1) & 2) === 0 ? size - 1 - vert : vert;
        if (!fn[y][x] && i < all.length * 8) { m[y][x] = ((all[i >>> 3] >>> (7 - (i & 7))) & 1) === 1; i++; }
      }
    }
    // The mask with the lowest penalty.
    const MASKS = [(x, y) => (x + y) % 2 === 0, (x, y) => y % 2 === 0, (x) => x % 3 === 0, (x, y) => (x + y) % 3 === 0,
      (x, y) => (Math.floor(x / 3) + Math.floor(y / 2)) % 2 === 0, (x, y) => (x * y) % 2 + (x * y) % 3 === 0,
      (x, y) => ((x * y) % 2 + (x * y) % 3) % 2 === 0, (x, y) => ((x + y) % 2 + (x * y) % 3) % 2 === 0];
    const apply = (k) => { for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) if (!fn[y][x] && MASKS[k](x, y)) m[y][x] = !m[y][x]; };
    const penalty = () => {
      let p = 0, dark = 0;
      const line = (get) => {
        for (let a = 0; a < size; a++) {
          let run = 1, s = "";
          for (let b = 0; b < size; b++) {
            const v = get(a, b); s += v ? "1" : "0";
            if (b > 0 && v === get(a, b - 1)) { run++; if (run === 5) p += 3; else if (run > 5) p++; } else run = 1;
          }
          for (let k = s.indexOf("1011101"); k >= 0; k = s.indexOf("1011101", k + 1)) {
            if (s.slice(Math.max(0, k - 4), k) === "0000" || (k < 4 && /^0*$/.test(s.slice(0, k)))) p += 40;
            else if (s.slice(k + 7, k + 11) === "0000" || (k + 11 > size && /^0*$/.test(s.slice(k + 7)))) p += 40;
          }
        }
      };
      line((a, b) => m[a][b]); line((a, b) => m[b][a]);
      for (let y = 0; y < size - 1; y++) for (let x = 0; x < size - 1; x++) { const c = m[y][x]; if (c === m[y][x + 1] && c === m[y + 1][x] && c === m[y + 1][x + 1]) p += 3; }
      for (const row of m) for (const v of row) if (v) dark++;
      return p + Math.max(0, Math.ceil(Math.abs(dark * 20 - size * size * 10) / (size * size)) - 1) * 10;
    };
    let best = 0, low = Infinity;
    for (let k = 0; k < 8; k++) { apply(k); format(k); const pk = penalty(); if (pk < low) { low = pk; best = k; } apply(k); }
    apply(best); format(best);
    return m;
  }

  // The global hotkey: the keys in use, and the others to pick from; a
  // line when another program has it (registering it failed).
  function hotkeyRow(st) {
    const choices = st.hotkeys || [];
    if (!choices.length) return null;
    const seg = ([code, keys]) => {
      const on = code === st.hotkey;
      const b = h("button", { class: "seg" + (on ? " on" : ""), type: "button", role: "radio", "aria-checked": String(on), id: "hk-" + code, dir: "ltr" }, keys);
      b.addEventListener("click", () => send({ op: "hotkey", key: code }));
      return b;
    };
    const row = h("div", { class: "srow hkrow" }, h("span", { class: "lbl", id: "hk-label" }, h("span", null, T.sHotkeyKeys), h("small", null, T.sHotkeyKeysSub)),
      h("div", { class: "segs", role: "radiogroup", "aria-labelledby": "hk-label" }, ...choices.map(seg)));
    if (!st.hotkey_taken) return row;
    return h("div", null, h("div", { class: "snote warn", id: "hk-taken", role: "alert" }, T.sHotkeyTaken(hotkeyText())), row);
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
  $("recent").addEventListener("keydown", (e) => {
    const r = S.recent[S.rsel], ctrl = e.ctrlKey || e.metaKey;
    if (!r) return;
    const moves = { ArrowDown: 1, ArrowUp: -1, Home: -1e9, End: 1e9 };
    if (e.key in moves) selectRecent(Math.max(0, Math.min(S.recent.length - 1, S.rsel + moves[e.key])), true);
    else if (e.key === "Enter" && e.altKey) act("properties", r, "recent");
    else if (e.key === "Enter") act(ctrl ? "reveal" : "open", r, "recent");
    else if (e.key === "F2") renameDialog(r, "recent");
    else if (e.key === "Delete" && !ctrl && !e.shiftKey && !e.altKey) recycleDialog(r, "recent");
    else if (ctrl && (e.key === "c" || e.key === "C")) act(e.shiftKey ? "copy" : "copy_file", r, "recent");
    else if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) openCtx(S.rsel, $("rr" + S.rsel), "recent");
    // Typing goes on in the search box.
    else if (!ctrl && !e.altKey && e.key.length === 1) { q.focus(); return; }
    else return;
    e.preventDefault(); e.stopPropagation();
  });
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
  $("tips").addEventListener("click", (e) => { const c = e.target.closest(".tq"); if (c) { q.value = c.textContent; q.focus(); runSearch(); } });
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
    if (e.key === "F1") { helpCard(); e.preventDefault(); return; }
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
    // A dialog waits and nothing was found: Enter moves nothing and starts
    // nothing (never "Search all drives"); it says what to do instead.
    if (S.qswitch && S.view === "nores") {
      if (e.key === "Enter" && inBox) { handlers.notice({ text: T.qsNone }); e.preventDefault(); }
      return;
    }
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
    // Alt+Enter: Windows' Properties window, as in Explorer.
    if (e.key === "Enter" && e.altKey && (inBox || inList)) { act("properties", row); e.preventDefault(); return; }
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
  $("tips").replaceChildren(...tipList().map(([c, s], i) => h("div", { class: "tip", role: "listitem" }, h("button", { class: "tq", type: "button", dir: "auto", "aria-describedby": "tip-d" + i }, c), h("span", { id: "tip-d" + i }, s))));
  $("tips").after(h("div", { class: "tips-more" }, btn("small", I.help, T.helpMore, helpCard, { id: "help-more" })));
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
