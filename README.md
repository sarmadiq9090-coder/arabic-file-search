<div dir="rtl">

# بحث سريع — برنامج بحث عن الملفات لويندوز

برنامج مفتوح المصدر مكتوب بلغة **Rust** للبحث السريع عن الملفات على جهازك، يدعم **اللغة العربية بالكامل**، ويبحث **داخل المستندات** أيضاً.

## المميزات

- **بحث فوري بالاسم**: يفهرس كل الأقراص مرة واحدة ويحفظ الفهرس، ثم تظهر النتائج أثناء الكتابة (ملي ثوانٍ حتى مع ملايين الملفات).
- **دعم كامل للعربية**: واجهة من اليمين لليسار بخط Segoe UI، والبحث يتجاهل الفروق بين (أ إ آ ا)، (ة ه)، (ى ي)، والتشكيل والتطويل، والأرقام العربية (٠١٢٣).
- **البحث داخل المستندات**: txt و docx و pptx و xlsx و xls و pdf و odt و ods و rtf و doc وملفات النصوص البرمجية وغيرها. يُظهر مقتطفاً من النص المطابق.
- **المسار الكامل** لكل ملف، مع الحجم وتاريخ التعديل.
- **فتح الملف** بنقرة مزدوجة أو Enter، و**فتح المجلد** مع تحديد الملف، و**نسخ المسار**، وقائمة بالزر الأيمن.
- **تحديد نطاق البحث**: كل الأقراص أو مجلد معيّن.

## طريقة البحث

| تكتب | المعنى |
|---|---|
| `تقرير المبيعات` | الملفات التي يحتوي اسمها على الكلمتين معاً |
| `"تقرير المبيعات"` | العبارة كما هي |
| `تقرير .pdf` أو `تقرير ext:pdf` أو `*.pdf` | ملفات PDF فقط |
| `ext:docx,xlsx` | أكثر من نوع |

للبحث داخل المستندات: فعّل خيار «البحث داخل المستندات أيضاً» ثم اضغط Enter أو زر «بحث». يمكنك إيقاف البحث في أي وقت.

## التحميل

حمّل `bahth.exe` من صفحة [Releases](../../releases) أو من نتائج GitHub Actions، وشغّله مباشرة (لا يحتاج تثبيت).
أول تشغيل يفهرس الأقراص (قد يستغرق دقيقة أو أكثر)، ويُحفظ الفهرس في `%LOCALAPPDATA%\BahthSari\index.bin`. اضغط «تحديث الفهرس» لتحديثه.

## البناء من المصدر

1. ثبّت Rust من <https://rustup.rs> (مع أدوات Visual Studio C++ Build Tools عند الطلب).
2. في مجلد المشروع:

```
cargo build --release
```

3. الملف الناتج: `target\release\bahth.exe`

للتشغيل المباشر: `cargo run --release` — وللاختبارات: `cargo test`.

البناء من لينكس (cross-compile): `rustup target add x86_64-pc-windows-gnu` وثبّت `mingw-w64` ثم `cargo build --release --target x86_64-pc-windows-gnu`.

## ملاحظات

- ملفات PDF الممسوحة ضوئياً (صور بلا نص) لا يمكن البحث داخلها.
- بعض ملفات PDF تخزّن العربية بترتيب معكوس فقد لا تُطابق.
- دعم ملفات doc القديمة تقريبي.

## هيكل الكود

| الملف | الوظيفة |
|---|---|
| `src/ui.rs` | الواجهة (Win32 أصلية عبر native-windows-gui، قائمة افتراضية سريعة) |
| `src/index.rs` | فهرسة الأقراص بالتوازي وحفظ الفهرس |
| `src/search.rs` | تحليل الاستعلام والبحث بالاسم وداخل المحتوى |
| `src/extract.rs` | استخراج النص من المستندات |
| `src/normalize.rs` | توحيد الحروف العربية للمطابقة |

## الرخصة

MIT — انظر ملف [LICENSE](LICENSE).

</div>

---

## English

**Bahth Sari'** is a fast, open-source Windows file search tool written in Rust with full Arabic support. It indexes all local drives into a cached in-memory index for instant filename search, and can search inside documents (txt, docx, pptx, xlsx/xls, pdf, odt/ods, rtf, doc, source files). Arabic matching ignores hamza forms, ta marbuta, alef maqsura, diacritics and tatweel. Double-click to open a file, open its folder, or copy its full path.

Build: `cargo build --release` on Windows (output `target\release\bahth.exe`). License: MIT.
