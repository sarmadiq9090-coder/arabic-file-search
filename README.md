<div dir="rtl">

<p align="center"><img src="assets/bahth-256.png" width="112" alt="أيقونة InstaFind — بحث سريع"></p>

<h1 align="center">InstaFind — بحث سريع</h1>

<p align="center"><b>أسرع طريقة لإيجاد أي ملف على ويندوز، بالعربية والإنكليزية، وحتى داخل المستندات.</b><br><b dir="ltr">Find any file on Windows as you type, in Arabic or English, even inside documents.</b></p>

<p align="center">
<a href="https://github.com/sarmadiq9090-coder/arabic-file-search/releases/latest"><img src="https://img.shields.io/github/v/release/sarmadiq9090-coder/arabic-file-search?label=%D8%A7%D9%84%D8%A5%D8%B5%D8%AF%D8%A7%D8%B1&color=2f81f7" alt="الإصدار / Release"></a>
<a href="https://github.com/sarmadiq9090-coder/arabic-file-search/releases"><img src="https://img.shields.io/github/downloads/sarmadiq9090-coder/arabic-file-search/total?label=%D8%A7%D9%84%D8%AA%D9%86%D8%B2%D9%8A%D9%84%D8%A7%D8%AA" alt="التنزيلات / Downloads"></a>
<a href="https://github.com/sarmadiq9090-coder/arabic-file-search/actions/workflows/build.yml"><img src="https://github.com/sarmadiq9090-coder/arabic-file-search/actions/workflows/build.yml/badge.svg" alt="CI"></a>
<a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-green" alt="MIT"></a>
</p>

<p align="center">
<a href="https://github.com/sarmadiq9090-coder/arabic-file-search/releases/latest/download/bahth.exe"><b>⬇ تنزيل bahth.exe · Download</b></a><br>
<a href="#العربية">العربية</a> · <a href="#english">English</a> · <a href="CHANGELOG.md">سجل التغييرات · Changelog</a>
</p>

<table align="center"><tr>
<td align="center">🌐 <b>البرنامج بلغتين: عربي و English</b><br>الواجهة تتبع لغة ويندوز، وتُغيَّر في أي وقت من «خيارات ← Language / اللغة»</td>
<td align="center" dir="ltr">🌐 <b>Two languages: English and Arabic</b><br>The interface follows Windows, and you can switch any time in Options → Language / اللغة</td>
</tr></table>

<p align="center">
<img src="docs/screenshots/search.png" width="49%" alt="نتائج فورية أثناء الكتابة">
<img src="docs/screenshots/inside-documents.png" width="49%" alt="البحث داخل المستندات مع مقتطف من النص">
</p>

## العربية

برنامج مجاني ومفتوح المصدر مكتوب بلغة Rust. يفهرس كل أقراصك مرة واحدة ثم يُظهر النتائج أثناء الكتابة، حتى مع ملايين الملفات. يفهم العربية كما تُكتب فعلاً: «احمد» تجد «أحمد»، و«التقرير» تجد «تقرير.pdf»، و`hgjrvdv` المكتوبة بلوحة المفاتيح الخطأ تجد «التقرير».

## التنزيل والتشغيل

1. نزّل [`bahth.exe`](https://github.com/sarmadiq9090-coder/arabic-file-search/releases/latest/download/bahth.exe) من [أحدث إصدار](https://github.com/sarmadiq9090-coder/arabic-file-search/releases/latest). ملف واحد، لا يحتاج تثبيتاً.
2. ضعه في مجلد دائم، مثلاً `%LOCALAPPDATA%\Programs\Bahth\` (هذا المسار تستعمله إضافات `integrations/`)، ثم شغّله.
3. إن ظهر تحذير SmartScreen لأن الملف غير موقّع رقمياً: «مزيد من المعلومات» ← «تشغيل على أي حال».

أول تشغيل يفهرس الأقراص في الخلفية (دقيقة أو أكثر حسب عدد الملفات)، وبعدها يفتح البرنامج فوراً ويبقى الفهرس محدَّثاً وحده عند إضافة الملفات أو حذفها أو إعادة تسميتها.

- **التحديث تلقائي**: يتحقق البرنامج مرة يومياً من GitHub ويعرض تثبيت الإصدار الجديد بنقرة ثم يعيد التشغيل، ويمكن التحقق يدوياً من «خيارات…». من لديه 0.3.1 أو أحدث يصله كل تحديث وحده.
- **المتطلبات**: ويندوز 8 أو أحدث، 64 بت. البحث داخل الصور يحتاج ويندوز 10 أو 11 (وحزمة اللغة العربية في ويندوز لقراءة النص العربي).
- **الخصوصية**: كل شيء يبقى على جهازك. الاتصال الوحيد بالإنترنت هو التحقق من التحديثات على GitHub، والبحث من الهاتف مطفأ ما لم تشغّله.
- **بيانات البرنامج** في `%LOCALAPPDATA%\BahthSari\` (الفهرس والسجل والإعدادات)، وتفضيلاته في `HKCU\Software\BahthSari`. لإزالته: أطفئ «التشغيل مع ويندوز» من «خيارات…»، ثم احذف `bahth.exe` وذلك المجلد.

## المميزات

### البحث
- **نتائج فورية بالاسم** أثناء الكتابة في كل الأقراص أو في مجلد تختاره، مع فلاتر جاهزة: مستندات، صور، صوت، فيديو، مضغوطات، برامج، مجلدات.
- **عربية حقيقية**: لا فرق بين أ إ آ ا، ولا ة ه، ولا ى ي، ويتجاهل التشكيل والتطويل و«ال»، ويقبل الأرقام العربية (٠١٢٣) والحروف الفارسية.
- **يسامح الأخطاء**: إن لم يجد شيئاً جرّب وحده لوحة المفاتيح الأخرى، ثم الفرانكو (`3abd` ← عبد)، ثم الكلمات القريبة إملائياً (`تقريز` ← التقرير). و`جذر:` يبحث بجذر الكلمة.
- **صيغة بحث قوية**: عبارة بين علامتي تنصيص، «أو» و«ليس»، نوع الملف، الحجم، تاريخ التعديل، التعبيرات المنتظمة، الكلمة كاملة، والتقارب بين كلمتين. انظر [الجدول أدناه](#طريقة-البحث).
- **الملفات الأكثر فتحاً أولاً**، وخط تحت الجزء المطابق، وترتيب بالنقر على عنوان أي عمود.
- **سجل البحث والبحوث المحفوظة** (Ctrl+H).

### داخل الملفات
- **داخل المستندات**: docx وxlsx وxls وpptx وpdf وodt وods وrtf وdoc وtxt وcsv، ورسائل البريد eml وmsg، وكتب epub، وملفات البرمجة، مع مقتطف من النص المطابق.
- **داخل الصور وملفات PDF الممسوحة ضوئياً** بمحرك التعرف على النصوص المدمج في ويندوز، دون أي تنزيل.
- **داخل الملفات المضغوطة** zip و7z وtar وtar.gz، تُقرأ في الذاكرة دون فك ضغطها، مع حماية من الأرشيفات المفخخة.
- **فهرس دائم للمحتوى**: يُستخرج نص كل مستند مرة واحدة في الخلفية، فيصبح البحث داخل الملفات بعدها شبه فوري.
- **كل التطابقات** داخل الملف مع أرقام الأسطر (F4)، وتصحيح العربية المعكوسة في ملفات PDF.

### أدوات
- **Ctrl+Alt+F** يُظهر البرنامج من أي مكان، و**Ctrl+Alt+G** في نافذة «حفظ» أو «فتح» بأي برنامج ينقلها إلى مجلد النتيجة التي تختارها.
- **الملفات المكررة**: يجدها بالحجم ثم بصمة المحتوى، ويعرض المساحة المهدرة، وينقل ما تختاره إلى سلة المحذوفات.
- **أحجام المجلدات**: عمود الحجم يشمل المجلدات، ونافذة تبيّن ما يملأ القرص.
- **عرض المصغرات** للصور والمستندات (Ctrl+T).
- **استبدال نص في عدة ملفات نصية** مع معاينة ونسخة احتياطية وزر تراجع.
- **البحث من الهاتف** عبر شبكة المنزل: مطفأ افتراضياً، برمز دخول، وللقراءة فقط.
- **سحب النتيجة وإفلاتها** في واتساب أو البريد أو Word، وCtrl+C ينسخ الملف نفسه، وF2 لإعادة التسمية، وDelete إلى سلة المحذوفات.
- **تصدير النتائج** إلى CSV يفتحه Excel بالعربية.
- **سطر الأوامر** و**إضافة Flow Launcher** وأمثلة PowerShell وAutoHotkey.

### الراحة
- **واجهة عربية أو إنكليزية**: تتبع لغة ويندوز، وتُغيَّر من «خيارات ← Language / اللغة».
- **يعمل مع ويندوز** في الخلفية، وEsc يخفيه إلى شريط المهام، ونسخة واحدة فقط منه.
- تواريخ نسبية («أمس»، «قبل 5 دقائق»)، وخيار الأرقام العربية، وأيقونات أنواع الملفات.
- أي خطأ داخلي يُسجَّل في `log.txt` و`crash.txt` ولا يُغلق البرنامج.

## طريقة البحث

| تكتب | المعنى |
|---|---|
| `تقرير المبيعات` | الاسم يحتوي الكلمتين معاً |
| `"تقرير المبيعات"` | العبارة كما هي |
| `تقرير .pdf` أو `ext:pdf` أو `*.pdf` | ملفات PDF فقط، و`ext:docx,xlsx` لأكثر من نوع |
| `صورة\|فيديو` | إحدى الكلمتين |
| `عقد !قديم` أو `عقد -قديم` | استبعاد ما يحتوي «قديم» |
| `مجلد:` أو `folder:` | المجلدات فقط |
| `حجم:>10م` أو `size:>10mb` | أكبر من 10 ميغابايت (`ب` `ك` `م` `ج` أو `b` `kb` `mb` `gb`)، و`size:1mb..5mb` بين حجمين |
| `حجم:فارغ` `صغير` `متوسط` `كبير` `ضخم` | أحجام جاهزة (`empty` `tiny` `small` `medium` `large` `huge` `gigantic`) |
| `تاريخ:اليوم` `أمس` `هذا_الأسبوع` `هذا_الشهر` `هذه_السنة` | حسب تاريخ التعديل (`dm:today` `yesterday` `thisweek` `thismonth` `thisyear`) |
| `dm:2024-05` أو `dm:>2024-01-01` أو `dm:7d` | شهر معيّن، أو بعد تاريخ، أو آخر 7 أيام (`3w` `2m` `1y`) |
| `regex:^IMG_\d+` | تعبير منتظم على الاسم (أو النص داخل المستندات) |
| `كلمة:عقد` أو `ww:عقد` | الكلمة كاملة: «عقد» لا «العقود» |
| `"عقد إيجار"~5` | الكلمتان وبينهما 5 كلمات على الأكثر |
| `تقريب: تقريز` أو `fuzzy:` | يتحمّل خطأً إملائياً أو خطأين |
| `فرانكو: 3abd` أو `franco:` | الحروف اللاتينية تُقرأ عربية |
| `جذر: كتاب` أو `root:` | «مكتبة» و«الكاتب» و«كتابات» |

للبحث داخل المستندات فعّل «البحث داخل المستندات أيضاً» ثم اضغط Enter. فلاتر الحجم والتاريخ تعمل في الحالتين، والأرقام العربية مقبولة في كل الفلاتر.

## لوحة المفاتيح

| المفتاح | العمل |
|---|---|
| Ctrl+Alt+F | إظهار البرنامج أو إخفاؤه من أي مكان |
| Ctrl+Alt+G | في نافذة حفظ أو فتح: الانتقال إلى مجلد نتيجة |
| Enter / Ctrl+Enter | فتح الملف / فتح مجلده مع تحديده |
| السهم للأسفل | من مربع البحث إلى النتائج (وفي مربع فارغ: السجل) |
| Ctrl+C / Ctrl+Shift+C | نسخ الملف نفسه / نسخ المسار |
| F2 / Delete | إعادة التسمية / النقل إلى سلة المحذوفات |
| F4 | كل التطابقات داخل الملف |
| Ctrl+H | السجل والبحوث المحفوظة |
| Ctrl+T | عرض المصغرات |
| Ctrl+F / Esc | العودة إلى مربع البحث / مسح البحث ثم الإخفاء |

## سطر الأوامر والتكامل

نفس `bahth.exe` يبحث من موجّه الأوامر بنفس الصيغة ومن الفهرس المحفوظ (افتح البرنامج مرة لبنائه):

```
bahth.exe -s "فاتورة ext:pdf"
bahth.exe -s تقرير -n 20 --sort date --desc --scope "D:\العمل"
bahth.exe -s عقد --json
```

الخيارات: `-n`/`--max`، و`--sort name|path|size|date` مع `--desc`، و`--scope`، و`--count`، و`--csv`، و`--json`، و`--help`. رمز الخروج 0 عند وجود نتائج، و1 عند عدمها، و2 عند الخطأ.

إضافة Flow Launcher جاهزة (اكتب `ب فاتورة`) وأمثلة PowerShell وAutoHotkey وWindows Terminal في [`integrations/`](integrations/).

## ملاحظات

- قراءة النص من الصور أبطأ من المستندات، وتحتاج حزمة لغة ويندوز لكل لغة (الإعدادات ← الوقت واللغة ← إضافة لغة). يُفحص أول 50 صفحة من كل PDF ممسوح، والصور الأكبر من 30 ميغابايت تُتجاهل.
- دعم ملفات doc القديمة تقريبي.
- المشاكل والاقتراحات مرحّب بها في [Issues](https://github.com/sarmadiq9090-coder/arabic-file-search/issues)، ويفيد إرفاق `%LOCALAPPDATA%\BahthSari\log.txt`.

## للمطوّرين

البناء من المصدر وخريطة الكود وطريقة النشر في [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md). باختصار: `cargo build --release` على ويندوز، و`cargo test` يعمل على لينكس أيضاً.

## الرخصة

[MIT](LICENSE)

</div>

---

## English

InstaFind is a free, open-source file search tool for Windows, written in Rust. It indexes all your drives once and shows results while you type, even with millions of files. It understands Arabic the way people actually type it: `احمد` finds `أحمد`, `التقرير` finds `تقرير.pdf`, and `hgjrvdv` typed on the wrong keyboard layout finds `التقرير`.

### Download and run

1. Download [`bahth.exe`](https://github.com/sarmadiq9090-coder/arabic-file-search/releases/latest/download/bahth.exe) from the [latest release](https://github.com/sarmadiq9090-coder/arabic-file-search/releases/latest). It is a single file; nothing to install.
2. Put it in a permanent folder, for example `%LOCALAPPDATA%\Programs\Bahth\` (the path the `integrations/` plugins expect), and run it.
3. If SmartScreen warns because the file is not code-signed: **More info → Run anyway**.

The first run indexes your drives in the background (a minute or more, depending on the number of files). After that the app opens instantly and keeps the index current by itself as files are added, deleted or renamed.

- **Automatic updates**: once a day the app checks GitHub, offers the new version with one click and restarts. You can also check from **Options…**. Version 0.3.1 and later update themselves.
- **Requirements**: Windows 8 or later, 64-bit. Text recognition in images needs Windows 10 or 11 (plus the Windows language pack for each OCR language).
- **Privacy**: everything stays on your PC. The only internet connection is the update check against GitHub; phone search is off unless you turn it on.
- **Data** lives in `%LOCALAPPDATA%\BahthSari\` (index, log, settings) and `HKCU\Software\BahthSari`. To remove the app, turn off *Start with Windows* in **Options…**, then delete `bahth.exe` and that folder.

### Features

#### Search
- **Instant filename search** across all drives or one folder, with ready-made filters: documents, pictures, audio, video, archives, programs, folders.
- **Real Arabic matching**: hamza forms, ta marbuta, alef maqsura, diacritics, tatweel and the `ال` prefix are ignored; Arabic-Indic digits and Persian letters are accepted.
- **Forgiving**: when nothing matches, it retries with the other keyboard layout, then Arabizi (`3abd` → عبد), then near spellings (one or two typos). `root:` searches by the Arabic triliteral root.
- **Rich query syntax**: phrases, OR, NOT, extensions, size, date modified, regex, whole word, and word proximity. See [Query syntax](#query-syntax).
- **Often-opened files first**, match underlining, sort by any column, search history and saved searches (Ctrl+H).

#### Inside files
- **Documents**: docx, xlsx, xls, pptx, pdf, odt, ods, rtf, doc, txt, csv, e-mail (eml, msg), epub and source code, with a snippet of the matching text.
- **Images and scanned PDFs** through the OCR engine built into Windows, with no download.
- **Archives**: zip, 7z, tar and tar.gz are read in memory without extracting, with zip-bomb limits.
- **Persistent content index**: each document's text is extracted once in the background, so later content searches are near-instant.
- **All matches** in a file with line numbers (F4), and reversed Arabic text in PDFs is repaired.

#### Tools
- **Ctrl+Alt+F** shows the app from anywhere; **Ctrl+Alt+G** in any program's Save or Open dialog jumps the dialog to the folder of a result.
- **Duplicate finder** (size, then content hash), sorted by wasted space, moves your picks to the Recycle Bin.
- **Folder sizes** in the Size column and a disk-usage window.
- **Thumbnail view** (Ctrl+T).
- **Find & replace in text files** with preview, backups and undo.
- **Search from your phone** over the home network: off by default, PIN-protected, read-only.
- **Drag & drop** results into WhatsApp, mail or Word; Ctrl+C copies the file itself; F2 renames; Delete sends to the Recycle Bin.
- **Export** results to CSV that opens correctly in Excel.
- **Command line**, a **Flow Launcher** plugin, and PowerShell / AutoHotkey examples.

#### Comfort
- **English or Arabic interface**: follows the Windows display language; switch in **Options → Language / اللغة**.
- **Starts with Windows** in the background, Esc hides it to the tray, single instance.
- Relative dates, optional Arabic digits, file-type icons.
- Internal errors are written to `log.txt` and `crash.txt` instead of closing the app.

### Query syntax

| Type | Meaning |
|---|---|
| `sales report` | names containing both words |
| `"sales report"` | the exact phrase |
| `report .pdf`, `ext:pdf` or `*.pdf` | PDF files only; `ext:docx,xlsx` for several |
| `photo\|video` | either word |
| `contract !old` or `contract -old` | exclude names containing `old` |
| `folder:` | folders only |
| `size:>10mb` | larger than 10 MB (`b` `kb` `mb` `gb`); `size:1mb..5mb` for a range |
| `size:empty` `tiny` `small` `medium` `large` `huge` `gigantic` | size presets |
| `dm:today` `yesterday` `thisweek` `thismonth` `thisyear` | by date modified |
| `dm:2024-05`, `dm:>2024-01-01`, `dm:7d` | a month, after a date, the last 7 days (`3w` `2m` `1y`) |
| `regex:^IMG_\d+` | regular expression on the name (or document text) |
| `ww:contract` | whole word only |
| `"lease contract"~5` | both words within 5 words of each other |
| `fuzzy: reprot` | tolerate one or two typos |
| `franco: ta2rer` | read Latin letters as Arabizi |
| `root: كتاب` | words sharing the Arabic root |

Every filter also has an Arabic spelling (`حجم:`, `تاريخ:`, `كلمة:`, `مجلد:` …). To search inside documents, tick **Also search inside documents** and press Enter.

### Keyboard

| Key | Action |
|---|---|
| Ctrl+Alt+F | show or hide the app from anywhere |
| Ctrl+Alt+G | in a Save/Open dialog: jump to a result's folder |
| Enter / Ctrl+Enter | open the file / open its folder with the file selected |
| Down arrow | from the search box to the results (in an empty box: history) |
| Ctrl+C / Ctrl+Shift+C | copy the file / copy the path |
| F2 / Delete | rename / move to the Recycle Bin |
| F4 | all matches inside the file |
| Ctrl+H | history and saved searches |
| Ctrl+T | thumbnail view |
| Ctrl+F / Esc | back to the search box / clear the search, then hide |

### Command line and integrations

The same `bahth.exe` searches from a terminal, with the same syntax, using the saved index (open the app once to build it):

```
bahth.exe -s "invoice ext:pdf"
bahth.exe -s report -n 20 --sort date --desc --scope "D:\Work"
bahth.exe -s contract --json
```

Options: `-n`/`--max`, `--sort name|path|size|date` with `--desc`, `--scope`, `--count`, `--csv`, `--json`, `--help`. Exit code 0 = results, 1 = none, 2 = error.

A Flow Launcher plugin (keyword `bs`) and PowerShell, AutoHotkey and Windows Terminal examples are in [`integrations/`](integrations/).

### Notes

- OCR is slower than document extraction. Only the first 50 pages of a scanned PDF are read, and images over 30 MB are skipped.
- Support for legacy `.doc` files is approximate.
- Bugs and ideas are welcome in [Issues](https://github.com/sarmadiq9090-coder/arabic-file-search/issues); attaching `%LOCALAPPDATA%\BahthSari\log.txt` helps.

### Developers

Building from source, the code map and the release process are in [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md). In short: `cargo build --release` on Windows; `cargo test` also runs on Linux.

### License

[MIT](LICENSE)
