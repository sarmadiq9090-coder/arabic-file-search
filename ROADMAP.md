# خارطة الطريق — InstaFind · Roadmap

<div dir="rtl">

**الدور:** رهان النمو الثاني: «برنامج Everything الذي يفهم العربية». الإطلاق الواسع (Show HN) يأتي بعد أعمال الثقة.
الخطة العامة لكل البرامج، وتعريف «البرنامج الناضج»، في [صفحة الحساب](https://github.com/sarmadiq9090-coder/sarmadiq9090-coder/blob/main/ROADMAP.md). وموعد كل بند يحدّده [معلَم قطار الجمعة](https://github.com/sarmadiq9090-coder/arabic-file-search/milestones) الذي يحمله (`train-YYYY-MM-DD`، الساعة 09:00 بتوقيت بغداد).

## المرحلة 1: الأساس (6 و13 نوفمبر)
- [ ] تثبيت إصدارات أدوات CI ببصمة الإيداع (SHA) وتفعيل Dependabot ([#19](https://github.com/sarmadiq9090-coder/arabic-file-search/pull/19)) ← قطار 6 نوفمبر.
- [ ] ملف `CLAUDE.md` بقواعد المالك ودليل المستودع ([#20](https://github.com/sarmadiq9090-coder/arabic-file-search/pull/20)) ← قطار 6 نوفمبر.
- [ ] قسم InstaFind في صفحة الخصوصية الموحّدة على fastkiwi.app.
- [ ] قائمة فحص الإصدار وخطة التراجع (في الأسفل).
- [ ] **على المالك:** إنشاء بيئة `release` (موافقة المالك، ووسوم `v*`). نحتاجها لاحقًا للتوقيع.

## المرحلة 2: الثقة (من 20 نوفمبر إلى 11 ديسمبر)
- [ ] **إصلاح أمني:** المحدِّث يرفض التحديث عندما لا ينشر GitHub بصمة SHA-256 للملف (`digest`)، بدل أن يثبّته بلا تحقق. يمكن أن يصدر قبل موعده بموافقة المالك.
- [ ] winget: لا نلمس الطلبات المقبولة، ونؤتمت التحديثات (wingetcreate) بعد دمجها. البرنامج المحمول يترك التحديث لـ winget وScoop.

## المرحلة 3: المستخدمون (ديسمبر ويناير)، ميزة واحدة فقط
- [ ] إظهار تقدّم الفهرسة في اليوم الأول، وخطوة «ثبّت البرنامج في مجلد دائم».
- [ ] بعدها: تحديثات موقّعة (minisign) كما في Lemon، في قطار مستقل.

## المرحلة 4: الوصول
- [ ] نوفمبر: AlternativeTo (كبديل لـ Everything)، وطلبات إضافة إلى awesome-arabic وawesome-windows.
- [ ] بعد الثقة: Show HN، وصفحة مقارنة «InstaFind أم Everything».

</div>

**Role:** second growth bet, "the Everything that understands Arabic". Show HN comes after the trust work.
The cross-app plan and the definition of "mature" are in the [profile roadmap](https://github.com/sarmadiq9090-coder/sarmadiq9090-coder/blob/main/ROADMAP.md). The [train milestones](https://github.com/sarmadiq9090-coder/arabic-file-search/milestones) (`train-YYYY-MM-DD`, Fridays 09:00 Baghdad) decide when each item ships.

| Phase | Items |
|---|---|
| **1. Foundation** (Nov 6, 13) | #19 CI pins + Dependabot and #20 CLAUDE.md (Nov 6 train); InstaFind section in the shared privacy page; release checklist + rollback; owner creates the `release` environment. |
| **2. Trust** (Nov 20 – Dec 11) | **Security fix:** the updater fails closed when the release has no SHA-256 `digest` (may ship early with owner approval). winget: leave validated PRs alone, then automate updates with wingetcreate. |
| **3. Users** (Dec – Jan) | One feature: first-day indexing progress plus an "install to a permanent folder" step. Then signed updates (minisign), on a train of their own. |
| **4. Reach** | November: AlternativeTo (as an Everything alternative), awesome-arabic and awesome-windows PRs. After trust: Show HN, an InstaFind vs Everything page. |

## قائمة فحص الإصدار والتراجع · Release checklist + rollback

<div dir="rtl">

الإصدار هنا يبدأ بدفع رقم إصدار جديد في `Cargo.toml` إلى `main`، فتنشئ مهمة `release` الوسم والإصدار بنفسها. الخطوات التفصيلية في [docs/DEVELOPMENT.md → Releases](docs/DEVELOPMENT.md#releases).

1. **CI ناجح:** كل فحوص `build.yml` خضراء على `main` (Linux وWindows والتجربة السريعة (smoke) واختبار الواجهة واختبار الشكل الجديد (newlook)).
2. **رقم الإصدار وCHANGELOG في يوم القطار فقط:** يُدمج رفع `version` و`CHANGELOG.md` (يصبح «غير منشور» «X.Y.Z — التاريخ») و`releases/vX.Y.Z-notes.md` في قطار InstaFind (السبت، بعد «موافق» من المالك)، ولا يصدر أكثر من إصدار في الأسبوع.
3. **التجربة بلا نشر:** لا يوجد تشغيل تجريبي منفصل. فحوص طلب الدمج تبني الملفات نفسها وتجرّبها على ويندوز حقيقي، فلا يُدمج رفع الإصدار قبل أن تنجح.
4. **قائمة الملفات:** الإصدار يحوي بالضبط 4 ملفات: `bahth.exe` و`bahth-x86.exe` و`bahth-arm64.exe` و`SHA256SUMS.txt`، وهو «Latest»، وليس مسودة ولا إصدارًا تجريبيًا (pre-release). لا تغيّر أسماء الملفات أبدًا. ملفات التوقيع `.minisig` تُضاف بعد إنشاء مفاتيح التوقيع (ابتداءً من 1.0.1).
5. **رقم الإصدار في الموقع:** في `site/index.html` و`site/en/index.html` غيّر `softwareVersion` وسطر «الإصدار X.Y.Z» و«الإصدار الحالي» وزر «تنزيل InstaFind X.Y.Z» إلى الرقم الجديد، في طلب الدمج نفسه (`pages.yml` ينشر الموقع عند الدمج).
6. **نسخة محدَّثة واحدة:** على جهاز فيه الإصدار السابق: **الإعدادات ← المزيد ← فتح قائمة الخيارات… ← التحقق من التحديثات** (أو «خيارات…» من أيقونة الساعة)، ثم تأكّد أن البرنامج يعمل بالرقم الجديد.

**التراجع عن إصدار سيئ:**
1. حوّل الإصدار السيئ إلى **مسودة** (draft): `gh release edit vX.Y.Z --draft=true -R sarmadiq9090-coder/arabic-file-search`. بعدها لا يراه المحدِّث، ولا صفحة `releases/latest`.
2. أصلح المشكلة، وأصدر **x.y.z+1** (مثلًا 0.9.1 بعد 0.9.0). لا تُعِد استخدام الرقم، ولا تحذف الوسم.
3. المحدِّث لا ينزل إلى إصدار أقدم أبدًا. من ثبّت الإصدار السيئ يبقى عليه حتى يصدر x.y.z+1، لذلك أصدره بسرعة.

</div>

Releasing here means pushing a new `version` in `Cargo.toml` to `main`. The `release` job then creates the tag and the release itself. The detailed steps are in [docs/DEVELOPMENT.md → Releases](docs/DEVELOPMENT.md#releases).

1. **CI green:** every `build.yml` job (Linux, Windows, smoke, UI test, new-look test) passes on `main`.
2. **Version bump and CHANGELOG on the train day only:** the `version`, `CHANGELOG.md` ("Unreleased" becomes "X.Y.Z — date") and `releases/vX.Y.Z-notes.md` change merges on InstaFind's train (Saturday, after the owner's «موافق»). At most one release a week.
3. **Dry run:** no separate dry run exists. The pull request's checks build and smoke-test the same exes on real Windows, so the bump merges only once they pass.
4. **Asset list:** exactly 4 assets: `bahth.exe`, `bahth-x86.exe`, `bahth-arm64.exe` and `SHA256SUMS.txt`. The release is marked Latest and is not a draft or pre-release. Never rename the assets. The `.minisig` signature files are added once the signing keys exist (from 1.0.1).
5. **Site version:** in `site/index.html` and `site/en/index.html` set `softwareVersion`, the "Version X.Y.Z" line, "The current version is" and the "Download InstaFind X.Y.Z" button to the new number, in the same pull request (`pages.yml` publishes the site on merge).
6. **One updated install:** on a PC with the previous version, run **Settings → More → Open the Options menu… → Check for updates** (or **Options…** from the tray icon), and confirm the new version starts.

**Rollback:**
1. Turn the bad release back into a **draft**: `gh release edit vX.Y.Z --draft=true -R sarmadiq9090-coder/arabic-file-search`. The updater and `releases/latest` stop offering it.
2. Fix the problem and ship **x.y.z+1**. Never reuse the number or delete the tag.
3. Updaters never downgrade. Installs that already took the bad version stay on it until x.y.z+1 ships, so ship it fast.
