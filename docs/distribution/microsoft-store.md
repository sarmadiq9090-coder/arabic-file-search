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
| `packaging/msix/make_layout.py` | One package folder per CPU, with the filled manifest, `LICENSE.txt` and `THIRD-PARTY-NOTICES.txt` |
| `packaging/make_notices.py` | Writes `THIRD-PARTY-NOTICES.txt` from `Cargo.lock` (`--check` in CI); rerun after a dependency change |
| `packaging/msix/pack.ps1` | `makepri` + `makeappx pack` per CPU, then `makeappx bundle` |
| `packaging/msix/test-install.ps1` | CI check: installs a test-signed copy and checks Store mode |
| `.github/workflows/store.yml` | Runs all of it on `windows-latest` and uploads the `store-package` artifact |
| `src/store.rs` | Store mode in the program (below) |

## Store mode in the program

When `GetCurrentPackageFullName` succeeds the program is the Store version:

- **Updates.** No GitHub check and no self-update (`update::managed_by()` returns `Store`, so `update.rs` refuses too). Options ▸ "Updates from Microsoft Store (version …)" (the same Options menu that the new look's Settings ▸ More opens) says that updates come from the Store and offers to open InstaFind's page in the Store app (`ms-windows-store://pdp/?ProductId=9MTK4KMFQ657`). Downloading code would break Store policy 10.2.2, and the program's folder (`WindowsApps`) is read-only anyway. Report a problem is unchanged (it names the install as "Microsoft Store").
- **Start with Windows** uses the package's StartupTask (`Windows.ApplicationModel.StartupTask`, TaskId `InstaFindStartup`) instead of the `Run` key, which a packaged app cannot write for real. It is **off until the user turns it on** (Options ▸ Start with Windows, the new look's Settings, or Settings ▸ Apps ▸ Startup ▸ InstaFind): `store::ENABLE_STARTUP_ON_FIRST_RUN` is `false` because Store policy 10.2.8 wants the user's consent (`autostart::apply_store_default`). If the user turns it off in Settings ▸ Apps ▸ Startup or in Task Manager, Windows reports `DisabledByUser`, the program never turns it back on, and the Options item explains where to change it. A sign-in launch is recognised by the activation kind (`StartupTask`), since the task passes no `--startup`, and the program starts hidden in the tray as before (classic and new look: `autostart::launched_at_sign_in`).
- **Data.** The program still writes to `%LOCALAPPDATA%\BahthSari` (index, content cache, rates, log, settings) and `HKCU\Software\BahthSari`. Windows redirects these writes to the package's private folder `%LOCALAPPDATA%\Packages\FastKiwi.InstaFind_sm1xjmj6v76yc\LocalCache\Local\BahthSari` and its private registry hive. The program sees one consistent view, and Windows removes the data on uninstall. The `instafind.exe` alias runs inside the package, so `instafind.exe -s "query"` reads the same index. A portable `bahth.exe` on the same PC keeps its own, separate index. The first log line of a Store run names the package and the real data folder. The index saver (temporary file, then rename; the dirty marker), settings, history, crash log, content cache, thumbnails and the new look's WebView2 folder all live there and work as in the portable program, because the program only ever uses the `%LOCALAPPDATA%\BahthSari` path. The file watcher and the document indexer see the real path, so they leave out both folders (`store::real_data_dir`): otherwise every log write would come back as a change, and the history would match every search inside documents.
- **No phone search.** Options has no "Search from phone" item and the LAN server never starts (it serves file names over plain HTTP; Store policy 10.5.4).
- **Licences.** The package carries `LICENSE.txt` and `THIRD-PARTY-NOTICES.txt` next to `bahth.exe`; About offers to open the notices (the portable program opens the copy on GitHub).
- **Unchanged:** global hotkeys (`RegisterHotKey`: Ctrl+Alt+F, Ctrl+Alt+G, optional Alt+Space), the tray icon, the floating box, the folder watchers and the indexer. A full-trust package runs as a normal desktop process of the user, and no low-level keyboard hooks are used. The single-instance mutex is shared with a portable copy, so only one of them runs at a time.

## 1. Fill the identity

Done: `packaging/msix/identity.json` has the Partner Center values. To check
or change them: Partner Center ▸ Apps and games ▸ InstaFind ▸ Product
management ▸ **Product identity**, then copy `Package/Identity/Name`,
`Package/Identity/Publisher` and `Package/Properties/PublisherDisplayName`.
`displayName` must equal the reserved name (`InstaFind`). Edit it in a PR. The
next `store` run then builds the real package.

## 2. Get the package

**Which release:** the first release that contains Store mode (`src/store.rs`) is **v1.0.0**. v0.10.0 and older have no Store mode: packaged, they would offer GitHub self-updates and use the `Run` key, so never package them for the Store.

- **Automatically:** when `build.yml` publishes a release, it starts `store.yml` with the new tag. The run is named `store v1.0.0` (for example) in the Actions list.
- **By hand:** GitHub ▸ Actions ▸ **store** ▸ Run workflow. Enter the tag (`v1.0.0` or later) to package that release's exact exes. Leave it empty only for a test build from the branch, never for an upload.
- Open the finished run. Its summary shows the file, the version and the identity (with a warning if it is a TEST identity). Download the **store-package** artifact and unzip it to get `InstaFind_X.Y.Z.0.msixbundle`.

The version is the app version plus `.0`, because the Store keeps the fourth number for itself. Every submission needs a higher version than the last, so submit after a release, never twice for the same version.

## 3. Upload it in Partner Center

1. Partner Center ▸ Apps and games ▸ InstaFind ▸ **Start submission** (later: **Update**).
2. **Pricing and availability:** Free; markets: all; visibility: public.
3. **Properties:** category **Utilities & tools** (subcategory *File managers*). Privacy policy URL `https://fastkiwi.app/privacy/`. Website `https://fastkiwi.app/arabic-file-search/`. Support contact: the GitHub issues URL. Product declarations: leave "accessibility" unticked unless tested. Tick nothing about pen/Xbox. System requirements: keyboard required; 4 GB RAM recommended.
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
> InstaFind finds files on Windows the moment you type, even with over a million files, and understands Arabic the way people really write it.
>
> Type "احمد" and it finds "أحمد". Type "التقرير" and it finds "تقرير". Typed with the wrong keyboard layout? "hgjrvdv" still finds "التقرير". Arabizi, typos and missing hamzas are handled too. English works just as well.
>
> It also searches inside documents: PDF, Word, Excel, PowerPoint, OpenDocument, RTF, e-mail, e-books and archives, and with Windows' own OCR the text in pictures and scanned PDFs.
>
> Press Ctrl+Alt+F anywhere for the quick search: files, apps (by their Arabic names too), Settings pages, calculations, units and currencies. Press Ctrl+Alt+G in any Open or Save dialog to jump to a folder you found.
>
> Private by design: the index is built and kept on your PC only. Nothing about your files or searches is ever sent anywhere.
>
> How it works: after installation InstaFind indexes the names of the files on your drives in the background, and the text inside common document types, and keeps the index up to date as files change. If you want, it can start with Windows in the tray: it is off until you turn it on in Options (or in Windows Settings ▸ Apps ▸ Startup). It goes online only to refresh currency exchange rates from open.er-api.com, at most every 12 hours. No account, no ads, no telemetry.

**Features** (one per line)
1. Results as you type, across over a million files
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
`file search`, `find files`, `Arabic search`, `search inside documents`, `instant file search`, `desktop search`, `بحث عن الملفات`

**Copyright:** © 2026 Sarmad (FastKiwi). All rights reserved.

## Store listing: Arabic (العربية)

**اسم المنتج:** InstaFind (الاسم المحجوز نفسه، بلا إضافة)

**وصف قصير**
> اعثر على أي ملف في حاسوبك أثناء الكتابة، بالعربية أو الإنجليزية، وحتى داخل المستندات. سريع وخاص ومجاني.

**الوصف**
> InstaFind يجد ملفاتك على ويندوز لحظة الكتابة، حتى بين أكثر من مليون ملف، ويفهم العربية كما نكتبها فعلاً.
>
> اكتب «احمد» فيجد «أحمد»، واكتب «التقرير» فيجد «تقرير»، ولو كتبت بلوحة المفاتيح الخطأ «hgjrvdv» يجد «التقرير». ويتعامل مع العربيزي والأخطاء الإملائية والهمزات الناقصة، والإنجليزية كذلك.
>
> ويبحث داخل المستندات: PDF وWord وExcel وPowerPoint وOpenDocument وRTF والبريد والكتب الإلكترونية والملفات المضغوطة، ويقرأ النص في الصور وملفات PDF الممسوحة بالتعرف الضوئي المدمج في ويندوز.
>
> اضغط Ctrl+Alt+F من أي مكان لنافذة البحث السريع: الملفات والبرامج (بأسمائها العربية أيضاً) وصفحات الإعدادات والحسابات والوحدات والعملات. واضغط Ctrl+Alt+G في أي نافذة فتح أو حفظ لتنتقل إلى المجلد الذي وجدته.
>
> خصوصيتك أولاً: الفهرس يُبنى ويُحفظ على جهازك وحده، ولا يُرسل أي شيء عن ملفاتك أو بحثك إلى أي مكان.
>
> كيف يعمل: بعد التثبيت يفهرس البرنامج أسماء الملفات في أقراصك في الخلفية، والنص داخل أنواع المستندات الشائعة، ويبقي الفهرس محدَّثاً عند تغيّر الملفات. ويمكنك أن تجعله يعمل مع بدء ويندوز في شريط المهام: الخيار مطفأ حتى تشغّله من «خيارات» (أو من إعدادات ويندوز ← التطبيقات ← بدء التشغيل). لا يتصل بالإنترنت إلا لتحديث أسعار صرف العملات من open.er-api.com، مرة كل 12 ساعة على الأكثر. بلا حساب ولا إعلانات ولا تتبّع.

**الميزات**
1. النتائج أثناء الكتابة بين أكثر من مليون ملف
2. مطابقة تفهم العربية: الهمزات والتاء المربوطة والألف المقصورة والتشكيل
3. يتحمل لوحة المفاتيح الخطأ والعربيزي والأخطاء الإملائية
4. البحث داخل PDF وOffice وOpenDocument والبريد والكتب الإلكترونية والملفات المضغوطة
5. النص في الصور وملفات PDF الممسوحة بالتعرف الضوئي في ويندوز
6. بحث سريع من أي مكان بـ Ctrl+Alt+F: ملفات وبرامج وإعدادات وإجابات
7. الانتقال إلى مجلد في أي نافذة فتح أو حفظ بـ Ctrl+Alt+G
8. فهرس حي يتبع تغييرات الملفات
9. كشف الملفات المكررة وأحجام المجلدات
10. واجهة عربية وإنجليزية تدعم الكتابة من اليمين إلى اليسار
11. كل شيء يبقى على جهازك: بلا حساب ولا إعلانات ولا تتبّع

**كلمات البحث**
`بحث عن الملفات`, `بحث سريع`, `البحث داخل المستندات`, `ايجاد ملف`, `file search`, `Arabic search`, `desktop search`

## Age rating (IARC questionnaire)

- Category: **Utilities & tools** (a utility app: not a game, not a browser, not social, not a store).
- Violence, fear, sexuality, nudity, profanity, drugs, alcohol, tobacco, gambling, crude humour, discrimination: **No** to all.
- Does the app let users interact or exchange content with other users? **No.** (The Store version has no phone search. Feedback opens a GitHub page in the user's browser.)
- Does it share the user's location with other users? **No.**
- Digital purchases? **No.** Ads? **No.**
- Unrestricted internet access (a browser or search engine)? **No.** Web shortcuts only open the user's default browser.
- Expected result: **3+ / Everyone** (PEGI 3, ESRB E).

## Notes for certification (paste into Partner Center)

> InstaFind is a desktop file-search utility (a Win32 program written in Rust) packaged as MSIX. It needs **runFullTrust** because it is a classic desktop app that:
> - indexes file and folder names on the user's local and removable drives in a background thread, and the text of common document types (PDF, Office, OpenDocument, text, e-mail, e-books, archives; images and scanned PDFs through Windows.Media.Ocr). It keeps the index current with ReadDirectoryChangesW folder watchers. The index lives only on the PC, in the package's own AppData; nothing about files or searches leaves the PC;
> - registers global hotkeys with RegisterHotKey: Ctrl+Alt+F opens its search window, Ctrl+Alt+G (in an Open/Save dialog) moves that dialog to a folder the user picks, Alt+Space is optional and off by default. No keyboard hooks are installed;
> - shows a notification-area (tray) icon and a small floating search box near the taskbar clock, both of which can be turned off in Options;
> - starts with Windows through its declared StartupTask (InstaFindStartup), minimised to the tray. It is **off by default**: the app never enables it by itself, only when the user turns it on in its Options menu ("Start with Windows") or in Settings > Apps > Startup > InstaFind. It can be turned off there or in Task Manager, and the app respects that choice (DisabledByUser).
>
> Network: the only automatic connection is a download of public currency exchange rates from https://open.er-api.com/v6/latest/USD (no key, no user data), at most every 12 hours, for the calculator's currency answers. The Store build never updates itself (updates come from the Store). "Send feedback" and the optional crash report only open a prefilled GitHub issue page in the user's browser; nothing is sent unless the user submits it there. The Store build has no "Search from phone" feature (the portable build's read-only home-network page uses plain HTTP, so it is removed from the Store build: no menu item, and the server never starts). The app opens no listening ports.
>
> The main window is drawn with Microsoft Edge WebView2 (the Evergreen runtime that Windows 10/11 already have); without it the app uses its classic Win32 window.
>
> To test: launch InstaFind and wait a minute for the first index (progress shows at the bottom). Type part of any file name, e.g. "windows" or "notepad", and the results appear as you type. Press Ctrl+Alt+F from any app for the quick search, and type "100 usd to eur" or "calc". Options > Language switches between Arabic and English. No account or sign-in is needed.

## Screenshots

The Store wants PNG screenshots of at least **1366×768** (1920×1080 is
best), between 1 and 10 per language. The current ones in `docs/screenshots/`
(1100×700) and `docs/media/` (946×593) are **too small**. Options:

1. **From CI (use these):** every `store` run has a `screenshots` job ([`ci/store-shots.ps1`](../../ci/store-shots.ps1)). It sets the runner's screen to 1920×1080, seeds sample files with Arabic and English names in `C:\Users\Public\Demo`, and captures full-screen shots in both languages: (1) name search («تقرير» / "report"); (2) search inside documents; (3) the Ctrl+Alt+F quick search; (4) the duplicate finder. The scenes use the classic window (the script's `settings.txt` says `new_ui=0`), although 1.0 opens the new look by default; for new-look shots use the `newlook-test` artifact of `build.yml` (`newlook.png`, `newlook-query.png`, 1100×700, too small for the Store as they are) or capture them by hand. They are uploaded as the **store-screenshots** artifact, together with `resolution.txt`, which records the resolution actually used. If the runner refused 1920×1080, the job warns and the shots are smaller (check that they are still at least 1366×768).
2. **By hand:** full-screen captures at 1920×1080 (Win+Shift+S).
3. **Stop-gap:** `python3 packaging/msix/make_assets.py --screenshots out/` puts each `docs/media/*.png` centred on a 1366×768 canvas. Certification accepts that, but it looks small in the Store.

### Captions (Partner Center ▸ Store listings ▸ Screenshots ▸ caption)

| Shot | English | العربية |
|---|---|---|
| `1-search-*.png` | Results as you type: Arabic names found however they are written | النتائج أثناء الكتابة، والأسماء العربية تُوجد كيفما كُتبت |
| `2-inside-*.png` | Search inside PDF, Word, Excel and text files, with the matching line | البحث داخل ملفات PDF وWord وExcel والنصوص، مع السطر المطابق |
| `3-quick-*.png` | Ctrl+Alt+F from anywhere: files, apps, settings and answers | Ctrl+Alt+F من أي مكان: ملفات وبرامج وإعدادات وإجابات |
| `4-duplicates-*.png` | Find duplicate files and free up space | اعثر على الملفات المكررة ووفّر المساحة |

Upload the `-en` shots to the English listing and the `-ar` shots to the Arabic one, in the order 1–4.

Logos for the listing: the Store builds the tile from the package. If Partner Center asks for a 1:1 "Store logo", upload `packaging/msix/Assets/Square150x150Logo.scale-400.png` (600×600) or render a 1080×1080 one with `assets/make_icon.py`.

## Store policy risks

| Risk | Policy | What we did / what to do |
|---|---|---|
| The indexer reads **every drive** (names) and **document contents** by default | 10.5.1 (personal information), 10.1 (accurate description) | Said plainly in the description and the certification notes; everything stays local. **To do:** make sure https://fastkiwi.app/privacy/ says InstaFind indexes local files and document text on the PC only, stores the index in its app data, and sends none of it. |
| **Exchange rates** fetched automatically from open.er-api.com | 10.5.1 (privacy policy covers network use) | Disclosed in the listing, the notes and the README (Arabic and English; the README used to say the update check was the only connection). The privacy policy must name open.er-api.com and say the request carries no user data (only the IP address the service sees). There is no off switch yet; adding one (Options) would make the disclosure stronger. |
| **Crash report / feedback** | 10.5.1 | Opt-in: they open a prefilled GitHub issue in the browser, and the user must press Submit. A crash report can contain file paths from the PC; the user sees the text before submitting. Mention it in the privacy policy. |
| **Self-update** | 10.2.2 (no dynamic code) | Off in the Store build (`store::is_packaged`: the menu shows the Store note, and `update::check_latest` never offers a release). |
| **Start with Windows** | 10.2.8 (user consent) | Opt-in: `ENABLE_STARTUP_ON_FIRST_RUN = false`, so the first run leaves the declared StartupTask off and `store.yml`'s install test fails if it is on. The user turns it on in Options or Settings ▸ Apps ▸ Startup, and Windows' `DisabledByUser` is respected. |
| **runFullTrust** (restricted capability) | Restricted capabilities review | Justified in the notes. Expected for a packaged Win32 app. |
| **Phone search** serves file names over plain HTTP on the LAN | 10.5.4 (secure transmission) | Not in the Store build: `web_server.rs` builds no menu item and never starts the server when `store::is_packaged()`. The portable build keeps it (off by default, PIN-protected, read-only). |
| **Third-party licences** | 10.1.x / open-source licence terms | Every package ships `LICENSE.txt` and `THIRD-PARTY-NOTICES.txt` (all 160 crates in the exe and their licence texts, and the OFL fonts of the new look, from `packaging/make_notices.py`; `store.yml` checks it matches `Cargo.lock`). About ▸ Yes opens it. |
| **Packaging unverified exes** | Supply chain | `store.yml` packs a release's exes only if each matches the SHA-256 digest GitHub recorded at upload, and fails closed otherwise. |
| **System commands** in the quick search (shut down, restart, lock) | 10.1 | Only when the user picks them. Mentioned implicitly via "answers"; fine. |
| **Screenshots** below 1366×768 | Listing requirements | Captured at 1920×1080 by `store.yml` (`store-screenshots`, above). |

## Notes for maintainers

- Store mode is decided once per process by `store::is_packaged()`. Any new feature that writes next to the exe, edits the `Run` key, installs or moves the program (for example a "move to a permanent folder" offer) must check it first.
- `store::is_packaged()` is the only package test: `rating::is_packaged()` (the rating question) and `update::managed_by()` (which then returns `PackageManager::Store`) use it.
- Keep `packaging/msix/AppxManifest.xml` and `src/store.rs` in step: `cargo test store::` checks the TaskId, alias, exe name, entry points, `runFullTrust` and MinVersion.
- Regenerate the logos after an icon change: `python3 packaging/msix/make_assets.py`.

## للمالك: خطوات النشر

**ما تحتاج أن تفعله أنت فقط** (بالترتيب):

1. **الهوية جاهزة:** وُضعت قيم Partner Center في `packaging/msix/identity.json` (الاسم `FastKiwi.InstaFind` والناشر `FastKiwi`). لا شيء عليك هنا.
2. **سياسة الخصوصية:** تأكد أن صفحة https://fastkiwi.app/privacy/ تذكر أن InstaFind:
   - يفهرس أسماء الملفات ونصوص المستندات على جهاز المستخدم فقط، ولا يرسلها إلى أي مكان؛
   - يجلب أسعار صرف العملات من open.er-api.com كل 12 ساعة على الأكثر، بلا أي بيانات من المستخدم؛
   - يفتح صفحة GitHub في المتصفح للملاحظات وتقارير الأعطال، ولا يرسل شيئاً إلا إذا ضغط المستخدم «Submit».
3. **الصور:** كل تشغيل لـ **store** يلتقط صور الشاشة بالعربية والإنجليزية ويضعها في **store-screenshots** أسفل صفحة التشغيل (بجانب store-package). نزّلها وارفعها في **Store listings**.
4. **الحصول على الحزمة:** بعد إصدار v1.0.0 تُبنى الحزمة تلقائياً، واسم التشغيل في Actions هو `store v1.0.0`. أو يدوياً:
   1. افتح صفحة المستودع على GitHub ← تبويب **Actions** ← اختر **store** من القائمة اليسرى.
   2. اضغط **Run workflow** ← اكتب رقم الإصدار في خانة tag: **`v1.0.0`** (أول إصدار فيه وضع المتجر) أو أحدث ← **Run workflow**. لا ترفع v0.10.0 أو أقدم إلى المتجر.
   3. انتظر حتى تظهر علامة ✓ خضراء، وافتح التشغيل، ثم نزّل **store-package** من أسفل الصفحة وفكّ ضغطه. ستجد ملف `InstaFind_….msixbundle`.
5. **الرفع إلى المتجر:**
   1. ادخل https://partner.microsoft.com ← **Apps and games** ← **InstaFind** ← **Start submission**.
   2. **Pricing and availability:** اختر **Free**.
   3. **Properties:** الفئة **Utilities & tools** (الفرعية **File managers**)، ورابط الخصوصية `https://fastkiwi.app/privacy/`.
   4. **Age ratings:** أجب «No» على كل الأسئلة (التفاصيل في قسم Age rating أعلاه). النتيجة المتوقعة 3+.
   5. **Packages:** اسحب ملف `.msixbundle` إلى الصفحة.
   6. **Store listings:** أضف **English** و**Arabic** وانسخ النصوص من القسمين أعلاه، وارفع الصور.
   7. **Submission options ← Notes for certification:** انسخ فقرة «Notes for certification» كما هي. وإن سُئلت عن **runFullTrust** فانسخ فقرتها الأولى.
   8. اضغط **Submit to the Store**. المراجعة تستغرق عادة من يوم إلى ثلاثة أيام عمل.
6. **كل تحديث لاحق:** بعد كل إصدار أسبوعي نزّل الحزمة الجديدة من Actions، ثم في Partner Center اضغط **Update** ← **Packages**، واستبدل الملف، ثم **Submit**. لا يمكن رفع الرقم نفسه مرتين.

**ملاحظات مهمة:**
- نسخة المتجر لا تحدّث نفسها؛ المتجر يحدّثها، وزر التحديث فيها يعرض فتح صفحة البرنامج في المتجر. وتشغيلها مع ويندوز يُضبط من «خيارات» أو من إعدادات ويندوز ← التطبيقات ← بدء التشغيل.
- بيانات نسخة المتجر منفصلة عن النسخة المحمولة (`bahth.exe`)، فلكل منهما فهرسها.
- نسخة المتجر لا تعمل مع ويندوز إلا إذا شغّل المستخدم ذلك بنفسه (شرط سياسة المتجر 10.2.8)، وتظهر في إعدادات ويندوز ← التطبيقات ← بدء التشغيل باسم InstaFind.
