<div dir="rtl">

# الخطة دفعةً بدفعة

القواعد: دفعة كل جمعة 09:00 بتوقيت بغداد، و**ميزة واحدة لكل دفعة**، ولا دفعة إلا وCI أخضر على لينكس وويندوز (ومنه `smoke` و`ui`). العمل خلال الأسبوع في فروع `claude/*`، ولا يُرفع رقم الإصدار إلا يوم الدفعة.
دفعتا 13 و20 نوفمبر محجوزتان لأعمال أخرى (0.9.2 من PR #34، وإصلاح التحديث «يفشل مغلقاً»)، ولا تدخلهما الواجهة الجديدة.

| الدفعة | ما يصل للمستخدمين (ميزة واحدة) | العمل قبلها في الفروع | شرط الخروج |
|---|---|---|---|
| **27 نوفمبر** | **البناء بـ MSVC وCRT ثابت** لـ `bahth.exe` و`bahth-x86.exe`، بالواجهة الحالية نفسها. لا تغيير يراه المستخدم | 9 إلى 25 نوفمبر: تعديل `build.yml` (البناء على `windows-latest`)، ومقارنة الحجم والسرعة مع نسخة mingw | `smoke` و`ui` خضراء على x64 وx86 وARM64، والتحديث الذاتي من 0.9.x يعمل، والملف لا يستورد أي DLL غير موجودة في ويندوز |
| **4 ديسمبر** | **الواجهة الجديدة (1.0.0) + الإطلاق في Microsoft Store معاً**: النافذة الرئيسية، ونافذة Ctrl+Alt+F، والإعدادات، وأول تشغيل، وحالات «فارغ / لا نتائج / أثناء الفهرسة». الواجهة القديمة بديل حين لا يوجد WebView2 | 9 إلى 15 نوفمبر: هيكل `src/webui/` والبروتوكول الخاص والنافذة المحمّلة مسبقاً، وقياس الميزانيات على ويندوز. 16 إلى 22: النافذة الرئيسية. 23 إلى 29: الصغيرة والإعدادات وأول تشغيل، ونقل `ci/ui-test.ps1` إلى UI Automation. 30 نوفمبر إلى 3 ديسمبر: حزمة المتجر (PR #26) بالواجهة الجديدة، وصور المتجر 1366×768 من النموذج الحقيقي، واختبار NVDA والراوي | الميزانيات كلها محققة في CI (150 / 50 ملّي ثانية، والذاكرة)، وكل فحوص Core خضراء، وفحص المسار العربي، والبديل يعمل على جهاز بلا WebView2 |
| **11 ديسمبر** | **السجل والبحوث المحفوظة** (Ctrl+H) في الواجهة الجديدة، مع نافذة «البحث من الهاتف» | — | لا تراجع في أي ميزانية |
| **18 ديسمبر** | **الملفات المكررة** في الواجهة الجديدة | — | نفس الفحوص الحالية للمكررات |
| **25 ديسمبر** | **أحجام المجلدات** (ما يملأ القرص) في الواجهة الجديدة | — | — |
| **1 يناير** | **استبدال النص في عدة ملفات** في الواجهة الجديدة. بعدها لا تبقى نافذة Win32 إلا البديل | — | — |
| **8 يناير** | **الوضع المضغوط والأعمدة** للمستخدم المحترف، و«كل التطابقات» (F4) داخل لوحة المعاينة | — | — |

## نقطة القرار قبل 4 ديسمبر

**الخميس 3 ديسمبر**: إن لم تكن شروط الخروج كلها خضراء، تتأجل الواجهة **والمتجر معاً** إلى 11 ديسمبر. لا نطلق المتجر بالواجهة القديمة ثم نغيّرها بعد أسبوع، لأن صور صفحة المتجر وأول انطباع يجب أن تكون للواجهة الجديدة.

## قرارات يحتاجها المالك

1. **اتجاه التصميم**: الألوان والخط والصف بسطرين ولوحة المعاينة ظاهرة افتراضياً. (نوصي: نعم.)
2. **الأيقونة الجديدة** (مربع أخضر وعدسة زرقاء ونقطة ليمونية) بدل الأيقونة الزرقاء الحالية. (نوصي: نعم، مع الواجهة في 4 ديسمبر، لتظهر في المتجر من أول يوم.)
3. **المتجر ينتظر الواجهة الجديدة** إلى 4 ديسمبر بدل الإطلاق بالواجهة الحالية. (نوصي: نعم.)
4. **رقم 1.0.0** للواجهة الجديدة. (نوصي: نعم.)
5. **مكان النافذة الصغيرة**: تبقى فوق الساعة كما اليوم، مع خيار «وسط الشاشة» في الإعدادات. (نوصي: فوق الساعة افتراضياً.)

</div>

## English

**Rules:** one feature per Friday train, green CI on Linux and Windows (including `smoke` and `ui`), and no version bump before release day. The Nov 13 and Nov 20 trains belong to other work (0.9.2 from #34, and the updater fail-closed fix).

| Train | Ships |
|---|---|
| Nov 27 | **MSVC + static CRT** for x64 and x86, with the current UI and no visible change. Gate: smoke/ui green on all three CPUs, self-update from 0.9.x works, no non-system DLL imports. |
| **Dec 4** | **New UI 1.0.0 together with the Microsoft Store launch.** It covers the main window, quick popup, settings, first run and the empty / no-results / indexing states, with the Win32 fallback when WebView2 is missing. Gate: every budget met in CI, all Core checks green, the RTL path check, and the fallback tested. |
| Dec 11 | History and saved searches, plus the phone-search window. |
| Dec 18 | Duplicate finder. |
| Dec 25 | Disk usage. |
| Jan 1 | Find and replace. After this, Win32 remains only as the fallback. |
| Jan 8 | Compact density, columns, and F4 all-matches in the preview pane. |

**Go/no-go on Thu Dec 3:** if any gate is red, the UI and the Store launch move together to Dec 11.

**Owner decisions:**
1. Design direction.
2. The new icon.
3. The Store waits for the new UI.
4. Version 1.0.0.
5. The popup stays above the clock by default.
