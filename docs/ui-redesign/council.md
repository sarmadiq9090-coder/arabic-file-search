<div dir="rtl">

# مراجعة المجلس: الواجهة الجديدة

نظر المجلس في صور الواجهة الحالية (`docs/media/*.png` و`docs/quick-shots/*.png`) وفي النموذج الجديد (`mockups/`).

## ما نحتفظ به (لا نقاش فيه)

- **السرعة**: النتائج تظهر أثناء الكتابة (15 إلى 34 ملّي ثانية اليوم). الواجهة الجديدة لا تُقبل إن أبطأت هذا.
- **العمل كله من لوحة المفاتيح**: الكتابة، والأسهم، وEnter، وTab للإجراءات، وEsc، وF2، وDelete، وCtrl+C، وF4، وCtrl+H، وCtrl+T.
- **البحث داخل المستندات** مع المقتطف المطابق ورقم السطر أو الصفحة.
- **نافذة Ctrl+Alt+F الصغيرة** في مكانها فوق الساعة، وخانة الكتابة في أسفلها، وخانة البحث العائمة.
- **الفهم العربي**: الهمزات، والتاء المربوطة، و«ال»، ولوحة المفاتيح الخطأ، والفرانكو، والجذر. هذا كله في المحرك ولا يتغيّر.

## الزوايا السبع

**تجربة الاستخدام.** المشكلة الأكبر اليوم ليست الشكل بل الترتيب: ثلاثة صفوف من الأزرار فوق النتائج («بحث»، «السجل»، «اختيار مجلد»، «كل الأقراص»، «تحديث الفهرس») وصف رابع تحتها، وكلها بالوزن نفسه. الخانة نفسها رفيعة، وزر «بحث» لا حاجة له لأن البحث يجري أثناء الكتابة. الحل: خانة واحدة كبيرة في الأعلى، والفلاتر رقائق صغيرة تحتها، والأفعال (فتح، المجلد، نسخ المسار) تنتقل إلى لوحة المعاينة وإلى Tab، وشريط الحالة يعرض المفاتيح المهمة. والحالات الفارغة («لا نتائج»، «أثناء الفهرسة»، «البحث فارغ») تصبح شاشات مفيدة: اقتراحات، وأين بحثنا، وماذا نجرّب. هذا معنى «لا شيء يضيع بصمت».

**الشكل.** الواجهة الحالية أدوات Win32 القياسية: مربعات بإطارات رمادية، وأزرار متساوية، وخطوط النظام، ومربعات فارغة بلا وظيفة بجانب «البحث داخل المستندات». الجديد يتبع هوية FastKiwi v2: أخضر الغابة للأفعال الأساسية وشريط التحديد، والليموني لتظليل الجزء المطابق، وأزرق السماء لون InstaFind (مفاتيح التشغيل، وحلقة التركيز، ولافتة الفهرسة)، وخط IBM Plex بالعربية واللاتينية. الوضعان الفاتح والداكن متساويان في العناية، ويتبعان ويندوز تلقائياً. الصفوف بسطرين (الاسم ثم المسار) مثل ما طلبه مستخدمو Everything، مع خيار «مضغوطة» بسطر واحد.

**المستخدم العربي.** أوضح عيب في الصور الحالية هو **المسار المقلوب**: `C:\Users\…\العمل\تقرير…` يظهر وأجزاؤه في غير مكانها، لأن الشرطة المائلة بين كلمتين عربيتين تأخذ اتجاه العربية. الحل في النموذج: المسار دائماً من اليسار لليمين، وكل اسم مجلد معزول وحده (`<bdi>`)، فلا تتبادل الأسماء العربية أماكنها. تحققنا من ذلك بقياس موضع كل جزء في المتصفح. الاسم نفسه يأخذ اتجاهه من أول حرف فيه، والمقتطفات بـ `dir="auto"`، والأرقام لاتينية افتراضياً مع خيار الأرقام العربية. والنصوص العربية مكتوبة بلغة طبيعية («عدد النتائج: 312 (خلال 18 ملّي ثانية)» كما في PR #34) وليست ترجمة حرفية. أزرار النافذة على اليسار في الواجهة العربية كما يفعل ويندوز، والرمز لا ينعكس أبداً.

**المستخدم المحترف الأجنبي.** يأتي من Everything وListary وPowerToys، ويريد: كثافة عالية، وأعمدة قابلة للترتيب، وصيغة بحث قوية، ولا شيء يقطع يده عن لوحة المفاتيح. لذلك: صيغة البحث الحالية كلها تبقى (`ext:`، `size:`، `dm:`، `regex:`، `ww:`)، وشاشة «البحث فارغ» تعلّمها بأمثلة، والقائمة تدعم الوضع المضغوط، وCtrl+K يركّز الخانة من أي مكان في النافذة. وأهم شيء له: المسارات الكاملة ظاهرة، تُختصر من الوسط لا من النهاية، ولا تختفي.

**الاختبار (QA).** سكربت `ci/ui-test.ps1` اليوم يقود أدوات Win32 بالرسائل، ونافذة البحث الصغيرة تكتب صفوفها في `quick-rows.txt`. مع WebView2 يجب أن يقرأ الاختبار الواجهة عبر UI Automation (Chromium يعرض شجرة إمكانية الوصول) أو عبر منفذ تصحيح يُفتح فقط حين يُضبط `BAHTH_UI_TEST`. شرط القبول: كل فحوص «Core» الحالية تبقى خضراء، وتُضاف فحوص جديدة للمسار العربي (ترتيب الأجزاء بالقياس كما في النموذج) ولقارئ الشاشة. ولقطات `docs/quick-shots/` تُعاد بالواجهة الجديدة.

**الأمان.** أسماء الملفات بيانات غير موثوقة: ملف اسمه `<img onerror=…>` يجب أن يظهر نصاً لا أن يُنفَّذ. لذلك: كل اسم يمر عبر `textContent` أو دالة تهريب (النموذج يفعل ذلك)، وسياسة CSP صارمة بلا سكربتات خارجية ولا `eval`، ومنع أي تنقّل خارج الواجهة (الروابط تفتح في المتصفح الافتراضي)، وأدوات المطوّر مطفأة في النسخة المنشورة. والواجهة لا تملك واجهة «اقرأ أي ملف»: تطلب «افتح النتيجة رقم 7» لا «افتح هذا المسار». والاتصال بين الواجهة والمحرك بلا منفذ شبكي مفتوح (التفاصيل في `architecture.md`). ولا نستعمل معالجات المعاينة الخارجية من ويندوز (Preview Handlers) لأنها تشغّل كود طرف ثالث داخل برنامجنا. المعاينة من النص الذي يستخرجه المحرك نفسه.

**الأداء.** المتصفح المدمج أثقل من Win32، والخطر الحقيقي في ثلاثة أماكن: وقت ظهور النافذة الصغيرة، والذاكرة في وضع الخمول، ونقل آلاف النتائج. الميزانيات (في `architecture.md`): النافذة الصغيرة مرئية خلال 150 ملّي ثانية من الضغط لأنها مُحمّلة مسبقاً ومخفية، والنتائج خلال 50 ملّي ثانية من آخر حرف، ولا تُرسل إلا الصفوف الظاهرة. الذاكرة: هدف واضح يُقاس في CI، وخفض استهلاك WebView2 حين تُخفى النافذة. وإذا لم تتحقق الميزانيات لا تخرج الدفعة.

## دروس من البرامج المشابهة

- **Everything (voidtools)**: قوته في السرعة والصيغة والأعمدة، وضعفه في الشكل. نسخة 1.5 أضافت شريط فلاتر وشريطاً جانبياً ولوحة معاينة، فاشتكى المستخدمون من عنوان لوحة المعاينة الذي يأكل المساحة، وطلبوا صفاً بسطرين (المسار في السطر الثاني). الدرس: المعاينة تُطفأ بمفتاح واحد ولا تأخذ مساحة بلا فائدة، والصف بسطرين.
- **Listary**: ميزته الكبرى «Quick Switch» في نوافذ الحفظ والفتح (عندنا Ctrl+Alt+G)، وأكثر شكاوى مستخدميه من شريط يظهر بلا تركيز على الكتابة أو يتأخر على الأقراص البطيئة. الدرس: النافذة الصغيرة يجب أن تأخذ التركيز فوراً وفي كل مرة، وأن تكون جاهزة في الذاكرة.
- **بحث ويندوز**: أكثر الشكاوى بطء، و«جارٍ العمل…» بلا نهاية، وملفات موجودة لا تظهر، وخلط نتائج الويب بالملفات، ونتائج تتحرك تحت المؤشر. الدرس: لا نتائج ويب إلا حين يطلبها المستخدم، ولا قائمة تقفز تحت الفأرة، وحين تكون الفهرسة ناقصة نقول ذلك صراحةً.
- **PowerToys Run وCommand Palette**: استبدلت Microsoft واجهة Run لأنها لم تعد تشبه ويندوز 11، وغيّرت الاختصار الافتراضي من Alt+Space إلى Win+Alt+Space لأن Alt+Space صار تأخذه برامج الذكاء الاصطناعي. الدرس: نبقي Ctrl+Alt+F افتراضياً، وAlt+Space اختيارياً كما اليوم، ونجعل الاختصار قابلاً للتغيير من الإعدادات.

</div>

## English

**Keep:** speed (results while typing), the full keyboard flow, search inside documents with snippets, the Ctrl+Alt+F popup above the clock, and the Arabic matching (all in the engine, unchanged).

- **UX:** one large search box, filter chips below it, actions in the preview pane and on Tab, and useful empty / no-results / indexing states instead of silent blanks ("nothing gets lost silently").
- **Visual:** FastKiwi v2 (forest, lime match highlight, sky accent, IBM Plex), with equal care for light and dark, two-line rows and an optional compact mode.
- **Arab user:** fix the reversed RTL paths. Paths are always LTR with each folder name isolated (`<bdi>`), verified by measuring segment positions. Names use the direction of their first letter, Arabic copy is natural, and caption buttons are mirrored.
- **Power user:** keep the whole query syntax, compact density, Ctrl+K focus, and full paths shortened in the middle.
- **QA:** port `ci/ui-test.ps1` to UI Automation (or a debug port opened only under `BAHTH_UI_TEST`). All current Core checks stay green, with new checks for RTL paths and the screen reader.
- **Security:** file names are untrusted, so escape them. Strict CSP, no navigation, devtools off, an id-based API (never "read any path"), no network port, and no third-party preview handlers.
- **Performance:** popup ≤150 ms (pre-warmed), results ≤50 ms, only the visible rows sent, a measured idle-RAM target, and no train if a budget fails.

**Lessons from similar apps:**
- **Everything 1.5:** users complained that the preview pane's title wastes space and asked for two-line rows.
- **Listary:** complaints about the bar not taking focus, or appearing late on slow disks.
- **Windows Search:** slowness, web results mixed in, and results moving under the cursor.
- **PowerToys:** Run was replaced because it looked dated, and the default moved away from Alt+Space.

Sources: [Everything forum: preview pane title](https://www.voidtools.com/forum/viewtopic.php?p=42354), [Everything forum: views and two-line items](https://www.voidtools.com/forum/viewtopic.php?p=55727), [Everything forum: filter bar](https://www.voidtools.com/forum/viewtopic.php?p=58255), [Listary Quick Switch](https://help.listary.com/quick-switch), [Listary 6 beta thread](https://discussion.listary.com/t/listary-6-beta/4615?page=18), [Command Palette overview](https://learn.microsoft.com/en-us/windows/powertoys/command-palette/overview), [Windows Latest on Command Palette](https://windowslatest.com/2025/04/01/powertoys-brings-command-palette-to-windows-11-and-its-better-than-built-in-search), [Microsoft Community: Windows 11 search slow](https://techcommunity.microsoft.com/discussions/windows11/windows-11-search-slow-very-very-slow/4500641), [Guiding Tech: slow search](https://www.guidingtech.com/top-ways-to-fix-slow-search-on-windows-11/).
