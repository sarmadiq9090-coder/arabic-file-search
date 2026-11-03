# InstaFind in the Microsoft Store

How the Store version is built and submitted. The same `bahth.exe` that GitHub
Releases publish goes into an MSIX package; the program notices it runs
packaged (`src/store.rs`) and behaves as a Store app.

| | |
|---|---|
| Store ID | `9MTK4KMFQ657` |
| Package/Identity/Name | `FastKiwi.InstaFind` |
| Publisher | `CN=8EFD6949-D088-481F-886D-82300679C5E5` |
| PublisherDisplayName | `FastKiwi` |
| Package family name | `FastKiwi.InstaFind_sm1xjmj6v76yc` |
| Package | `InstaFind_<version>.msixbundle` (x64, x86, arm64), unsigned: the Store signs it |
| Minimum Windows | 10.0.17763.0 (Windows 10 1809) |
| Privacy policy | https://fastkiwi.app/privacy/ |

**القسم العربي للمالك في الأسفل:** [للمالك: خطوات النشر](#للمالك-خطوات-النشر)

## What is in the repository

| Path | What it is |
|---|---|
| `packaging/msix/AppxManifest.xml` | Manifest template: Windows.Desktop, `runFullTrust`, the `instafind.exe` alias, the `InstaFindStartup` startup task |
| `packaging/msix/identity.json` | Partner Center identity. A value with `TODO` makes the build use the `test` block and say so |
| `packaging/msix/Assets/` | Logos at scales 100–400 and target sizes 16–256, made by `make_assets.py` from the app icon (`assets/make_icon.py`, the source of `assets/bahth-256.png`) |
| `packaging/msix/make_layout.py` | One package folder per CPU, with the filled manifest |
| `packaging/msix/pack.ps1` | `makepri` + `makeappx pack` per CPU, then `makeappx bundle` |
| `packaging/msix/test-install.ps1` | CI check: installs a test-signed copy and checks Store mode |
| `.github/workflows/store.yml` | Runs all of it on `windows-latest` and uploads the `store-package` artifact |
| `src/store.rs` | Store mode in the program (below) |

## Store mode in the program

When `GetCurrentPackageFullName` succeeds the program is the Store version:

- **Updates.** No GitHub check and no self-update (`update.rs` refuses too). Options ▸ "Updates from Microsoft Store (version …)" says that updates come from the Store. Downloading code would break Store policy 10.2.2, and the program's folder (`WindowsApps`) is read-only anyway.
- **Start with Windows** uses the package's StartupTask (`Windows.ApplicationModel.StartupTask`, TaskId `InstaFindStartup`) instead of the `Run` key, which a packaged app cannot write for real. The first run turns it on once, like the portable program does (`store::ENABLE_STARTUP_ON_FIRST_RUN`). If the user turns it off in Settings ▸ Apps ▸ Startup or in Task Manager, Windows reports `DisabledByUser`, the program never turns it back on, and the Options item explains where to change it. A sign-in launch is recognised by the activation kind (`StartupTask`), since the task passes no `--startup`, and the program starts hidden in the tray as before.
- **Data.** The program still writes to `%LOCALAPPDATA%\BahthSari` (index, content cache, rates, log, settings) and `HKCU\Software\BahthSari`. Windows redirects these writes to the package's private folder `%LOCALAPPDATA%\Packages\FastKiwi.InstaFind_sm1xjmj6v76yc\LocalCache\Local\BahthSari` and its private registry hive. The program sees one consistent view, and Windows removes the data on uninstall. The `instafind.exe` alias runs inside the package, so `instafind.exe -s "query"` reads the same index. A portable `bahth.exe` on the same PC keeps its own, separate index. The first log line of a Store run names the package and the real data folder.
- **Unchanged:** global hotkeys (`RegisterHotKey`: Ctrl+Alt+F, Ctrl+Alt+G, optional Alt+Space), the tray icon, the floating box, the folder watchers and the indexer. A full-trust package runs as a normal desktop process of the user, and no low-level keyboard hooks are used. The single-instance mutex is shared with a portable copy, so only one of them runs at a time.

## 1. Fill the identity

Done: `packaging/msix/identity.json` has the Partner Center values. To check
or change them: Partner Center ▸ Apps and games ▸ InstaFind ▸ Product
management ▸ **Product identity**, then copy `Package/Identity/Name`,
`Package/Identity/Publisher` and `Package/Properties/PublisherDisplayName`.
`displayName` must equal the reserved name (`InstaFind`). Edit it in a PR. The
next `store` run then builds the real package.

## 2. Get the package

- **Automatically:** when `build.yml` publishes a release, it starts `store.yml` with the new tag.
- **By hand:** GitHub ▸ Actions ▸ **store** ▸ Run workflow. Enter a tag (e.g. `v0.9.1`) to package that release's exact exes, or leave it empty to build from the branch.
- Open the finished run. Its summary shows the file, the version and the identity (with a warning if it is a TEST identity). Download the **store-package** artifact and unzip it to get `InstaFind_X.Y.Z.0.msixbundle`.

The version is the app version plus `.0`, because the Store keeps the fourth number for itself. Every submission needs a higher version than the last, so submit after a release, never twice for the same version.

## 3. Upload it in Partner Center

1. Partner Center ▸ Apps and games ▸ InstaFind ▸ **Start submission** (later: **Update**).
2. **Pricing and availability:** Free; markets: all; visibility: public.
3. **Properties:** category *Productivity* (subcategory none), or *Utilities & tools*. Privacy policy URL `https://fastkiwi.app/privacy/`. Website `https://fastkiwi.app/arabic-file-search/`. Support contact: the GitHub issues URL. Product declarations: leave "accessibility" unticked unless tested. Tick nothing about pen/Xbox. System requirements: keyboard required; 4 GB RAM recommended.
4. **Age ratings:** the questionnaire, answers below.
5. **Packages:** drag in the `.msixbundle`. Partner Center validates it (identity, version, `runFullTrust`). Device families: Windows 10/11 Desktop.
6. **Store listings:** add **English (United States)** and **Arabic**, and paste the texts below. Upload screenshots and the 1:1 logo.
7. **Submission options ▸ Notes for certification:** paste the notes below. **Restricted capabilities:** justify `runFullTrust` with the first paragraph of those notes.
8. **Submit to the Store.** Certification usually takes 1–3 working days.

## Store listing: English

**Product name:** InstaFind

**Short description**
> Find any file on your PC as you type, in Arabic or English, and search inside documents. Fast, private, free.

**Description**
> InstaFind finds files on Windows the moment you type, even across millions of files, and understands Arabic the way people really write it.
>
> Type "احمد" and it finds "أحمد". Type "التقرير" and it finds "تقرير". Typed with the wrong keyboard layout? "hgjrvdv" still finds "التقرير". Arabizi, typos and missing hamzas are handled too. English works just as well.
>
> It also searches inside documents: PDF, Word, Excel, PowerPoint, OpenDocument, RTF, e-mail, e-books and archives, and with Windows' own OCR the text in pictures and scanned PDFs.
>
> Press Ctrl+Alt+F anywhere for the quick search: files, apps (by their Arabic names too), Settings pages, calculations, units and currencies. Press Ctrl+Alt+G in any Open or Save dialog to jump to a folder you found.
>
> Private by design: the index is built and kept on your PC only. Nothing about your files or searches is ever sent anywhere.
>
> How it works: after installation InstaFind indexes the names of the files on your drives in the background, and the text inside common document types, and keeps the index up to date as files change. It can start with Windows in the tray (you can turn this off in Options or in Windows Settings). It goes online only to refresh currency exchange rates from open.er-api.com, at most every 12 hours. No account, no ads, no telemetry.

**Features** (one per line)
1. Results as you type, across millions of files
2. Arabic-aware matching: hamza, ta marbuta, alef maqsura, diacritics
3. Wrong keyboard layout, Arabizi and typo tolerance
4. Search inside PDF, Office, OpenDocument, e-mail, e-books and archives
5. Text in images and scanned PDFs with Windows OCR
6. Quick search anywhere with Ctrl+Alt+F: files, apps, settings, answers
7. Jump to a folder in any Open/Save dialog with Ctrl+Alt+G
8. Live index that follows file changes
9. Duplicate finder and folder sizes
10. Arabic and English interface, right-to-left aware
11. Everything stays on your PC: no account, no ads, no telemetry

**Search terms** (7 max)
`file search`, `find files`, `Arabic search`, `search inside documents`, `everything search`, `desktop search`, `بحث عن الملفات`

**Copyright:** © 2026 Sarmad (FastKiwi). MIT licensed.

## Store listing: Arabic (العربية)

**اسم المنتج:** InstaFind — بحث سريع (إن قبل Partner Center اسماً محلياً؛ وإلا InstaFind)

**وصف قصير**
> اعثر على أي ملف في حاسوبك أثناء الكتابة، بالعربية أو الإنكليزية، وحتى داخل المستندات. سريع وخاص ومجاني.

**الوصف**
> «بحث سريع» يجد ملفاتك على ويندوز لحظة الكتابة، حتى بين ملايين الملفات، ويفهم العربية كما نكتبها فعلاً.
>
> اكتب «احمد» فيجد «أحمد»، واكتب «التقرير» فيجد «تقرير»، ولو كتبت بلوحة المفاتيح الخطأ «hgjrvdv» يجد «التقرير». ويتعامل مع العربيزي والأخطاء الإملائية والهمزات الناقصة، والإنكليزية كذلك.
>
> ويبحث داخل المستندات: PDF وWord وExcel وPowerPoint وOpenDocument وRTF والبريد والكتب الإلكترونية والملفات المضغوطة، ويقرأ النص في الصور وملفات PDF الممسوحة بالتعرف الضوئي المدمج في ويندوز.
>
> اضغط Ctrl+Alt+F من أي مكان لنافذة البحث السريع: الملفات والبرامج (بأسمائها العربية أيضاً) وصفحات الإعدادات والحسابات والوحدات والعملات. واضغط Ctrl+Alt+G في أي نافذة فتح أو حفظ لتنتقل إلى المجلد الذي وجدته.
>
> خصوصيتك أولاً: الفهرس يُبنى ويُحفظ على جهازك وحده، ولا يُرسل أي شيء عن ملفاتك أو بحثك إلى أي مكان.
>
> كيف يعمل: بعد التثبيت يفهرس البرنامج أسماء الملفات في أقراصك في الخلفية، والنص داخل أنواع المستندات الشائعة، ويبقي الفهرس محدَّثاً عند تغيّر الملفات. ويمكن أن يعمل مع بدء ويندوز في شريط المهام (يُطفأ من «خيارات» أو من إعدادات ويندوز). لا يتصل بالإنترنت إلا لتحديث أسعار صرف العملات من open.er-api.com، مرة كل 12 ساعة على الأكثر. بلا حساب ولا إعلانات ولا تتبّع.

**الميزات**
1. النتائج أثناء الكتابة بين ملايين الملفات
2. مطابقة تفهم العربية: الهمزات والتاء المربوطة والألف المقصورة والتشكيل
3. يتحمل لوحة المفاتيح الخطأ والعربيزي والأخطاء الإملائية
4. البحث داخل PDF وOffice وOpenDocument والبريد والكتب الإلكترونية والملفات المضغوطة
5. النص في الصور وملفات PDF الممسوحة بالتعرف الضوئي في ويندوز
6. بحث سريع من أي مكان بـ Ctrl+Alt+F: ملفات وبرامج وإعدادات وإجابات
7. الانتقال إلى مجلد في أي نافذة فتح أو حفظ بـ Ctrl+Alt+G
8. فهرس حي يتبع تغييرات الملفات
9. كشف الملفات المكررة وأحجام المجلدات
10. واجهة عربية وإنكليزية تدعم الكتابة من اليمين إلى اليسار
11. كل شيء يبقى على جهازك: بلا حساب ولا إعلانات ولا تتبّع

**كلمات البحث**
`بحث عن الملفات`, `بحث سريع`, `البحث داخل المستندات`, `ايجاد ملف`, `file search`, `Arabic search`, `desktop search`

## Age rating (IARC questionnaire)

- Category: **Productivity / utility app** (not a game, not a browser, not social, not a store).
- Violence, fear, sexuality, nudity, profanity, drugs, alcohol, tobacco, gambling, crude humour, discrimination: **No** to all.
- Does the app let users interact or exchange content with other users? **No.** (Search from phone is a read-only page for the user's own devices on their home network, off by default. Feedback opens a GitHub page in the user's browser.)
- Does it share the user's location with other users? **No.**
- Digital purchases? **No.** Ads? **No.**
- Unrestricted internet access (a browser or search engine)? **No.** Web shortcuts only open the user's default browser.
- Expected result: **3+ / Everyone** (PEGI 3, ESRB E).

## Notes for certification (paste into Partner Center)

> InstaFind is a desktop file-search utility (a Win32 program written in Rust) packaged as MSIX. It needs **runFullTrust** because it is a classic desktop app that:
> - indexes file and folder names on the user's local and removable drives in a background thread, and the text of common document types (PDF, Office, OpenDocument, text, e-mail, e-books, archives; images and scanned PDFs through Windows.Media.Ocr). It keeps the index current with ReadDirectoryChangesW folder watchers. The index lives only on the PC, in the package's own AppData; nothing about files or searches leaves the PC;
> - registers global hotkeys with RegisterHotKey: Ctrl+Alt+F opens its search window, Ctrl+Alt+G (in an Open/Save dialog) moves that dialog to a folder the user picks, Alt+Space is optional and off by default. No keyboard hooks are installed;
> - shows a notification-area (tray) icon and a small floating search box near the taskbar clock, both of which can be turned off in Options;
> - starts with Windows through its declared StartupTask (InstaFindStartup), minimised to the tray. It is enabled on first run and can be turned off in the app's Options menu, in Settings > Apps > Startup or in Task Manager; the app respects that choice.
>
> Network: the only automatic connection is a download of public currency exchange rates from https://open.er-api.com/v6/latest/USD (no key, no user data), at most every 12 hours, for the calculator's currency answers. The Store build never updates itself (updates come from the Store). "Send feedback" and the optional crash report only open a prefilled GitHub issue page in the user's browser; nothing is sent unless the user submits it there. "Search from phone" (a PIN-protected, read-only page on the home network) is off by default and only starts when the user turns it on, which may show a Windows Firewall prompt.
>
> To test: launch InstaFind and wait a minute for the first index (progress shows at the bottom). Type part of any file name, e.g. "windows" or "notepad", and the results appear as you type. Press Ctrl+Alt+F from any app for the quick search, and type "100 usd to eur" or "calc". Options > Language switches between Arabic and English. No account or sign-in is needed.

## Screenshots

The Store wants PNG screenshots of at least **1366×768** (1920×1080 is
best), between 1 and 10 per language. The current ones in `docs/screenshots/`
(1100×700) and `docs/media/` (946×593) are **too small**. Options:

1. **Best:** take new full-screen captures at 1920×1080 on Windows (Win+Shift+S, or the `ui` CI job's step screenshots in the `ui-test` artifact if the runner's screen is big enough). Suggested set, in both languages: (1) main window with Arabic results for «تقرير»; (2) search inside documents with a matched line; (3) the Ctrl+Alt+F quick search with apps and an answer; (4) the floating box near the clock; (5) the duplicate finder.
2. **Stop-gap:** `python3 packaging/msix/make_assets.py --screenshots out/` puts each `docs/media/*.png` centred on a 1366×768 canvas. Certification accepts that, but it looks small in the Store.

Logos for the listing: the Store builds the tile from the package. If Partner Center asks for a 1:1 "Store logo", upload `packaging/msix/Assets/Square150x150Logo.scale-400.png` (600×600) or render a 1080×1080 one with `assets/make_icon.py`.

## Store policy risks

| Risk | Policy | What we did / what to do |
|---|---|---|
| The indexer reads **every drive** (names) and **document contents** by default | 10.5.1 (personal information), 10.1 (accurate description) | Said plainly in the description and the certification notes; everything stays local. **To do:** make sure https://fastkiwi.app/privacy/ says InstaFind indexes local files and document text on the PC only, stores the index in its app data, and sends none of it. |
| **Exchange rates** fetched automatically from open.er-api.com | 10.5.1 (privacy policy covers network use) | Disclosed in the listing, the notes and the README (Arabic and English; the README used to say the update check was the only connection). The privacy policy must name open.er-api.com and say the request carries no user data (only the IP address the service sees). There is no off switch yet; adding one (Options) would make the disclosure stronger. |
| **Crash report / feedback** | 10.5.1 | Opt-in: they open a prefilled GitHub issue in the browser, and the user must press Submit. A crash report can contain file paths from the PC; the user sees the text before submitting. Mention it in the privacy policy. |
| **Self-update** | 10.2.2 (no dynamic code) | Off in the Store build (`store::is_packaged`: the menu shows the Store note, and `update::check_latest` never offers a release). |
| **Start with Windows turned on at first run** | User control of startup behaviour | It goes through the declared StartupTask, can be turned off three ways, and Windows' `DisabledByUser` is respected. If certification objects, set `ENABLE_STARTUP_ON_FIRST_RUN = false` in `src/store.rs` and it becomes opt-in. |
| **runFullTrust** (restricted capability) | Restricted capabilities review | Justified in the notes. Expected for a packaged Win32 app. |
| **Phone search** opens a LAN server | 10.5 / security | Off by default, PIN-protected, read-only, user-initiated. |
| **System commands** in the quick search (shut down, restart, lock) | 10.1 | Only when the user picks them. Mentioned implicitly via "answers"; fine. |
| **Screenshots** below 1366×768 | Listing requirements | New captures needed (above). |

## Notes for maintainers

- Store mode is decided once per process by `store::is_packaged()`. Any new feature that writes next to the exe, edits the `Run` key, installs or moves the program (for example a "move to a permanent folder" offer) must check it first.
- PR #21/#25 add `update::managed_by()` for winget/Scoop installs; the Store guard in `update.rs` is one separate `if crate::store::is_packaged()` block at the top of `check_latest` and merges with them and with #24.
- Keep `packaging/msix/AppxManifest.xml` and `src/store.rs` in step: `cargo test store::` checks the TaskId, alias, exe name, entry points, `runFullTrust` and MinVersion.
- Regenerate the logos after an icon change: `python3 packaging/msix/make_assets.py`.

## للمالك: خطوات النشر

**ما تحتاج أن تفعله أنت فقط** (بالترتيب):

1. **الهوية جاهزة:** وُضعت قيم Partner Center في `packaging/msix/identity.json` (الاسم `FastKiwi.InstaFind` والناشر `FastKiwi`). لا شيء عليك هنا.
2. **سياسة الخصوصية:** تأكد أن صفحة https://fastkiwi.app/privacy/ تذكر أن «بحث سريع»:
   - يفهرس أسماء الملفات ونصوص المستندات على جهاز المستخدم فقط، ولا يرسلها إلى أي مكان؛
   - يجلب أسعار صرف العملات من open.er-api.com كل 12 ساعة على الأكثر، بلا أي بيانات من المستخدم؛
   - يفتح صفحة GitHub في المتصفح للملاحظات وتقارير الأعطال، ولا يرسل شيئاً إلا إذا ضغط المستخدم «Submit».
3. **الصور:** نحتاج صوراً للشاشة بحجم 1366×768 أو أكبر (الأفضل 1920×1080). الصور الحالية أصغر من ذلك. اطلب من جلسة Claude التقاطها، أو التقطها أنت بـ Win+Shift+S.
4. **الحصول على الحزمة:** بعد إصدار الجمعة تُبنى الحزمة تلقائياً. أو يدوياً:
   1. افتح صفحة المستودع على GitHub ← تبويب **Actions** ← اختر **store** من القائمة اليسرى.
   2. اضغط **Run workflow** ← اكتب رقم الإصدار في خانة tag (مثلاً `v0.9.1`) ← **Run workflow**.
   3. انتظر حتى تظهر علامة ✓ خضراء، وافتح التشغيل، ثم نزّل **store-package** من أسفل الصفحة وفكّ ضغطه. ستجد ملف `InstaFind_….msixbundle`.
5. **الرفع إلى المتجر:**
   1. ادخل https://partner.microsoft.com ← **Apps and games** ← **InstaFind** ← **Start submission**.
   2. **Pricing and availability:** اختر **Free**.
   3. **Properties:** الفئة **Productivity**، ورابط الخصوصية `https://fastkiwi.app/privacy/`.
   4. **Age ratings:** أجب «No» على كل الأسئلة (التفاصيل في قسم Age rating أعلاه). النتيجة المتوقعة 3+.
   5. **Packages:** اسحب ملف `.msixbundle` إلى الصفحة.
   6. **Store listings:** أضف **English** و**Arabic** وانسخ النصوص من القسمين أعلاه، وارفع الصور.
   7. **Submission options ← Notes for certification:** انسخ فقرة «Notes for certification» كما هي. وإن سُئلت عن **runFullTrust** فانسخ فقرتها الأولى.
   8. اضغط **Submit to the Store**. المراجعة تستغرق عادة من يوم إلى ثلاثة أيام عمل.
6. **كل تحديث لاحق:** بعد كل إصدار أسبوعي نزّل الحزمة الجديدة من Actions، ثم في Partner Center اضغط **Update** ← **Packages**، واستبدل الملف، ثم **Submit**. لا يمكن رفع الرقم نفسه مرتين.

**ملاحظات مهمة:**
- نسخة المتجر لا تحدّث نفسها؛ المتجر يحدّثها. وتشغيلها مع ويندوز يُضبط من «خيارات» أو من إعدادات ويندوز ← التطبيقات ← بدء التشغيل.
- بيانات نسخة المتجر منفصلة عن النسخة المحمولة (`bahth.exe`)، فلكل منهما فهرسها.
- إن رفضت مراجعة المتجر تشغيل البرنامج مع ويندوز تلقائياً عند أول تشغيل، فهناك سطر واحد يجعله اختيارياً (`ENABLE_STARTUP_ON_FIRST_RUN`)، واطلب ذلك من جلسة Claude.
