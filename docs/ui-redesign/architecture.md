<div dir="rtl">

# ملاحظة البنية: الواجهة الجديدة على WebView2

## الفكرة في سطرين

**المحرك يبقى كما هو**: الفهرسة، والبحث، والمطابقة العربية، واستخراج النص، والمراقبة، والتحديث (`src/index.rs`، `src/search/`، `src/extract/`، `src/launcher/`…).
**الواجهة وحدها تنتقل**: من native-windows-gui إلى نافذة tao فيها WebView2 عبر wry، والصفحات HTML/CSS/JS مدمجة داخل `bahth.exe` نفسه. يبقى البرنامج ملفاً واحداً.

```
bahth.exe
 ├─ المحرك (Rust، بلا تغيير)  ← خيوط الفهرسة والبحث والمراقبة
 ├─ طبقة الواجهة (جديدة): src/webui/
 │    ├─ protocol.rs   بروتوكول خاص يقدّم الصفحات وواجهة JSON (لا منفذ شبكي)
 │    ├─ bridge.rs     رسائل الواجهة ← المحرك، والنتائج ← الواجهة
 │    ├─ windows.rs    نافذتا tao: الرئيسية والصغيرة (المحمّلة مسبقاً)
 │    └─ assets/       الصفحات والخطوط (Plex) مدمجة في الملف التنفيذي
 └─ الواجهة القديمة (src/ui/) تبقى بديلاً حين لا يوجد WebView2
```

## الاتصال بين الواجهة والمحرك (IPC)

درسنا خيارين:

| | بروتوكول خاص في wry | خادم محلي على 127.0.0.1 |
|---|---|---|
| منفذ شبكي مفتوح | لا | نعم (يحتاج حارس Host/Origin) |
| خطر DNS rebinding أو برنامج آخر يتصل | لا يوجد، فالطلبات لا تغادر WebView2 | موجود، ويعالجه الحارس والرمز |
| البث المستمر للنتائج | `window.ipc.postMessage` للطلبات، و`evaluate_script` لدفع النتائج | يحتاج WebSocket أو SSE |
| خبرة سابقة عندنا | — | Lemon (`server/guard.rs`) |

**القرار المقترح: البروتوكول الخاص.** wry يقدّمه على ويندوز بعنوان `http://instafind.localhost/` لكن WebView2 يعترض الطلب داخلياً ولا يصل إلى الشبكة. ونضيف فوقه، للحماية المضاعفة:
- **رمز لكل تشغيل** (عشوائي 128 بت، يُولَّد عند البدء ويُحقن في الصفحة عند تحميلها) يُرسل مع كل طلب JSON، ويُرفض أي طلب بدونه.
- **فحص Origin**: لا يُقبل إلا `http://instafind.localhost`.
- **منع التنقل**: أي رابط خارجي يفتح في المتصفح الافتراضي، ولا تُفتح نوافذ جديدة داخل البرنامج.
- **CSP صارمة**: `default-src 'self'; script-src 'self'; object-src 'none'; base-uri 'none'`، ولا `eval`، ولا خطوط أو سكربتات من الإنترنت.
- **واجهة بالمعرّفات**: الواجهة تقول «افتح النتيجة 7 من البحث 42»، والمحرك يعرف المسار. لا توجد دالة «اقرأ هذا المسار» أو «شغّل هذا الأمر».
- **أدوات المطوّر والقائمة الافتراضية** مطفأة في النسخة المنشورة.

إن احتجنا الخادم المحلي لاحقاً (مثلاً للاختبار) فبنفس حارس Lemon: Host/Origin يساوي `127.0.0.1:<منفذ>` أو `localhost:<منفذ>`، مع رمز لكل تشغيل. وخادم «البحث من الهاتف» الحالي (`src/web.rs`) منفصل ويبقى كما هو: مطفأ افتراضياً، برمز PIN، للقراءة فقط.

## ميزانيات الأداء (تُقاس في CI على ويندوز حقيقي، وتمنع الدفعة إن فشلت)

| المقياس | الهدف | كيف |
|---|---|---|
| ظهور نافذة Ctrl+Alt+F | **≤ 150 ملّي ثانية** من الضغط حتى ظهور الخانة مع التركيز فيها | النافذة الصغيرة تُنشأ عند بدء البرنامج، وتُحمَّل صفحتها، وتبقى مخفية. الضغط = `ShowWindow` + تركيز + رسالة «افتح» للصفحة. لا تحميل ولا تحليل HTML عند الضغط |
| النتائج بعد الكتابة | **≤ 50 ملّي ثانية** من آخر حرف حتى رسم الصفوف (المحرك وحده اليوم 15 إلى 34) | البحث في خيط منفصل مع إلغاء البحث القديم (رقم جيل)، وإرسال أول 60 صفاً فقط، والباقي عند التمرير (قائمة افتراضية)، وJSON مختصر |
| بدء البرنامج | النافذة الرئيسية خلال **≤ 600 ملّي ثانية** في التشغيل الدافئ | بيئة WebView2 واحدة مشتركة للنافذتين (عملية متصفح واحدة)، والصفحات مدمجة لا تُقرأ من القرص |
| الذاكرة في الخمول | الهدف **≤ 150 ميغابايت** مجموع عمليات البرنامج وWebView2 والنوافذ مخفية، ولا تتجاوز اليوم + 100 ميغابايت | `MemoryUsageTargetLevel = Low` حين تُخفى النافذة، و`TrySuspend` للنافذة الرئيسية المخفية (لا للصغيرة)، وتحرير النافذة الرئيسية بعد 10 دقائق مخفية |
| حجم الملف | زيادة ≤ 3 ميغابايت | wry وtao والخطوط (Plex عربي ولاتيني بثلاثة أوزان ≈ 300 كيلوبايت) |

تجربة سابقة في هذه الجلسة: برنامج wry 0.57.1 + tao 0.37.2 بأبسط صفحة عربية حجمه 3.9 ميغابايت بـ GNU.

## حين لا يوجد WebView2

- ويندوز 11 فيه WebView2 دائماً. ومعظم أجهزة ويندوز 10 حصلت عليه مع تحديثات Edge، لكن ليس كلها.
- عند البدء: `GetAvailableCoreWebView2BrowserVersionString`. إن لم يوجد، **تعمل الواجهة الحالية (Win32) كما هي اليوم** دون أي فقدان في الوظائف، ويظهر سطر واحد في الإعدادات: «الواجهة الجديدة تحتاج WebView2 من Microsoft» مع زر يفتح صفحة Microsoft الرسمية. لا تنزيل ولا تثبيت تلقائي.
- إن فشل إنشاء WebView2 أثناء التشغيل (عملية المتصفح انهارت): نسجّل في `log.txt`، ونعيد المحاولة مرة، ثم نرجع إلى الواجهة القديمة. لا يُغلق البرنامج أبداً.
- الواجهة القديمة تبقى في الكود 3 أشهر على الأقل بعد الإطلاق، ثم يقرر المالك.

## إمكانية الوصول

- **لوحة المفاتيح فقط**: كل شيء يعمل بلا فأرة. خانة البحث `role=combobox` مع `aria-activedescendant`، فالتركيز يبقى في الخانة والأسهم تنقل التحديد (كما في النموذج). Tab ينتقل إلى الرقائق ثم المعاينة، وحلقة التركيز ظاهرة دائماً.
- **قارئ الشاشة**: Chromium يعرض شجرة UI Automation، فيعمل NVDA والراوي. القائمة `role=listbox` وكل صف `role=option` باسم مقروء («تقرير المبيعات 2026.xlsx، في المستندات\العمل، 48 كيلوبايت، قبل 7 دقائق»)، وعدد النتائج في `aria-live=polite` دون إزعاج مع كل حرف (ينتظر 500 ملّي ثانية من آخر حرف). نختبر بالعربية والإنجليزية.
- **التباين العالي**: `forced-colors` يُحترم. الألوان تمر بفحص WCAG AA (أزرق السماء لا يُستعمل نصاً على الأبيض، بل `#0A6A96`).
- **التكبير**: Ctrl+ وCtrl− حتى 200% دون قص، واحترام إعداد «تقليل الحركة».

## الأسماء المجمّدة (من CLAUDE.md، لا تتغير)

- أسماء الملفات المنشورة: `bahth.exe` (x64، كل نسخة مثبتة منذ 0.3.0 تبحث عنه)، و`bahth-x86.exe`، و`bahth-arm64.exe`.
- المستودع `sarmadiq9090-coder/arabic-file-search` (في `src/update.rs` والحزم ودلو Scoop).
- البيانات `%LOCALAPPDATA%\BahthSari` والتفضيلات `HKCU\Software\BahthSari`. **مجلد بيانات WebView2 يوضع داخله**: `%LOCALAPPDATA%\BahthSari\WebView2`، لا بجانب الملف التنفيذي (الافتراضي، ويفشل في Program Files). وفي نسخة المتجر داخل مجلد الحزمة الخاص.
- هوية المتجر من PR #26: `FastKiwi.InstaFind`، والاسم المستعار `instafind.exe`، ومهمة البدء `InstaFindStartup`.
- عنوان النافذة «InstaFind — بحث سريع» (PR #34): البحث عن النسخة الوحيدة يعتمد عليه، فتحمله نافذة tao أيضاً.
- المجلدات: `ci/` و`bucket/` و`releases/` و`packaging/` و`docs/media/` و`docs/quick-shots/` و`vendor/native-windows-gui` و`assets/`.

## الانتقال إلى MSVC مع CRT ثابت: دفعة كاملة قبل الواجهة

- wry على ويندوز يعتمد على `webview2-com`. مع هدف GNU (MinGW) يرتبط البرنامج بالملف `WebView2Loader.dll`، وهذا يكسر قاعدة «ملف واحد». **تحققنا**: البرنامج التجريبي (wry 0.57.1، `x86_64-pc-windows-gnu`) يستورد `WebView2Loader.dll` حسب `objdump -p`.
- مع MSVC يرتبط بالمكتبة الثابتة `WebView2LoaderStatic.lib`، فلا ملف إضافي. ومع `-C target-feature=+crt-static` لا يحتاج حزمة Visual C++. نسخة ARM64 تُبنى اليوم بهذه الطريقة (`.cargo/config.toml`).
- لذلك: **دفعة كاملة قبل الواجهة** تنقل `bahth.exe` و`bahth-x86.exe` من بناء mingw على لينكس إلى MSVC على `windows-latest`، بالواجهة الحالية نفسها. إن ظهرت مشكلة (مكافح فيروسات، أو اختلاف في الأداء، أو ربط) نراها وحدها، لا مختلطة مع الواجهة الجديدة.
- يبقى `linux-test` كما هو (المحرك يُختبر على لينكس). خيار احتياطي للبناء من لينكس: `cargo-xwin`، يُجرَّب ولا يُعتمد إلا إذا نجح `smoke` و`ui`.

</div>

## English

**Shape.** The Rust engine is unchanged. The UI moves from native-windows-gui to tao windows hosting WebView2 through wry. Pages are embedded in `bahth.exe`, so it stays a single file. The Win32 UI remains as the fallback.

**IPC.** Recommended: a wry custom protocol, served as `http://instafind.localhost/` and intercepted inside WebView2, so no network port is opened. On top of it:
- a per-launch 128-bit token on every JSON call, plus an Origin check;
- `ipc.postMessage` for requests, and `evaluate_script` to push results;
- navigation blocked, and new windows sent to the default browser;
- a strict CSP with no `eval` and no remote assets;
- an id-based API ("open result 7 of query 42", never "read this path");
- devtools off in release.

A localhost server, if ever needed, uses Lemon's Host/Origin guard plus the token. The phone-search server stays as it is.

**Budgets (measured on real Windows in CI; a miss blocks the train):**
- quick popup visible with focus ≤150 ms, via a pre-warmed hidden window;
- results ≤50 ms after the last keystroke, with a worker thread, generation-based cancellation and a virtualized list (first 60 rows);
- warm start ≤600 ms;
- idle RAM ≤150 MB in total, and no more than +100 MB over today, using `MemoryUsageTargetLevel=Low`, suspending the hidden main view and releasing it after 10 minutes;
- exe growth ≤3 MB.

**No WebView2.** Detect it with `GetAvailableCoreWebView2BrowserVersionString`. If it is missing, run today's Win32 UI and show a one-line note that links to Microsoft's page. Nothing is downloaded automatically. On a runtime failure, log it, retry once, then fall back. The old UI stays for at least 3 months.

**Accessibility:**
- combobox and listbox with `aria-activedescendant`, a full keyboard path and a visible focus ring;
- names that UIA can read, for NVDA and Narrator in Arabic and English;
- a debounced live region for the result count;
- `forced-colors`, WCAG AA contrast, zoom to 200% and reduced motion.

**Frozen names:**
- the exe and asset names, the repository URL, `%LOCALAPPDATA%\BahthSari` and `HKCU\Software\BahthSari`;
- the WebView2 user-data folder goes under `BahthSari\WebView2`, not next to the exe;
- the Store identity, alias and startup task from #26;
- the window title used by the single-instance lookup from #34;
- the protected folders.

**MSVC first.** wry/webview2-com on the GNU target imports `WebView2Loader.dll`. We verified this with `objdump` on a wry 0.57.1 spike, and it breaks the one-exe rule. MSVC links `WebView2LoaderStatic.lib`, and with `+crt-static` it needs no VC++ redistributable (ARM64 already builds this way). So one full train moves x64 and x86 to MSVC on `windows-latest` with the current UI, before the new UI ships. `linux-test` stays. `cargo-xwin` is an optional Linux fallback, adopted only if `smoke` and `ui` pass.
