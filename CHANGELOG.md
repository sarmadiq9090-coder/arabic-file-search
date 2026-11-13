<div dir="rtl">

# سجل التغييرات

## غير منشور (Unreleased)

<!-- pending: on release day rename this heading to «## 1.0.0 — <date>» in the train PR (with the Cargo.toml bump); until then it stays «Unreleased». -->

هذا هو الإصدار 1.0.0 القادم. كل سطر بالعربية أولاً ثم بالإنجليزية بعد «/». / The coming 1.0.0. Each line: Arabic first, then English after "/".

### الذاكرة والثبات / Memory and stability
- **لا تكدّس في الذاكرة عند حفظ الفهرس (بلاغ 5 غيغابايت)**: على جهاز تتغيّر ملفاته طوال الوقت (تصيير، محرك ألعاب، تنزيل كبير) كان كل حفظ ينتظر دوره ومعه نسخة كاملة من الفهرس، فتتراكم نسخة جديدة كل نصف دقيقة والمعالج شبه متوقف، حتى بلغت الذاكرة 5 غيغابايت عند أحد المستخدمين. الآن يحفظ خيط واحد، حفظاً واحداً في كل مرة، ولا يأخذ نسخة الفهرس إلا لحظة الكتابة، فلا توجد أكثر من نسختين (التي تُكتب والتي تستعملها النافذة). الحفظ البطيء يؤخّر الحفظ التالي، والحفظ الفاشل يُعاد وحده بفترات متزايدة، ولا يُكتب فهرس أقدم فوق أحدث. / **No memory pile-up from index saves (the 5 GB report):** on a PC whose files change all the time, every waiting save held a full copy of the index, and one more piled up every half minute at almost no CPU. Now one saving thread writes one save at a time and takes the index only when its write starts, so at most two copies exist; a slow save spaces out the next one, a failed one is retried with growing pauses, and an older index never overwrites a newer one.
- **ملف الفهرس سليم حتى مع نسختين من البرنامج**: كل نسخة تكتب ملفها المؤقت الخاص (`index.<pid>.tmp`) ثم تضعه مكان `index.bin`، وتعيد المحاولة إن كان الملف مشغولاً، وإن فشلت يبقى الفهرس القديم ويُعاد الحفظ لاحقاً. بقايا الحفظ غير المكتمل تُحذف عند التشغيل. / **The index file stays whole with two copies running:** each process writes its own `index.<pid>.tmp` and renames it over `index.bin`, retrying a busy file; on failure the old index is kept and the save is tried again later. Leftovers of unfinished saves are removed at start.
- **التغييرات غير المحفوظة لا تضيع بصمت**: ما دامت في الذاكرة تغييرات لم تُكتب يوجد الملف `index.dirty`. إذا انتهى البرنامج قبل حفظها (إنهاء قسري، انقطاع الكهرباء) يفحص التشغيل التالي الأقراص من جديد في الخلفية. والفحص الكامل الذي فشل حفظه يُحفظ لاحقاً. / **Unsaved changes are not lost silently:** `index.dirty` exists while memory holds changes not on disk; a start that finds it scans the drives again in the background. A full scan whose save failed is saved later.
- **طوابير محدودة لتغييرات الملفات**: التغييرات التي لا يمكن تطبيقها فوراً (أثناء البحث داخل المستندات مثلاً) كانت تنتظر بلا حد، فقد تضيف مئات الميجابايتات في الساعة على جهاز يكتب ملفات باستمرار. الآن لكل طابور حدّ (200,000 تغيير أو نحو 64 ميجابايت)، وما يتجاوزه يُستبدل بفحص واحد للقرص. / **Bounded change queues:** watcher changes that cannot be applied yet wait in queues of at most 200,000 changes or about 64 MB each; an overflow becomes one paced walk of that drive.
- **فحص القرص كاملاً أخف**: نسختان على الأكثر من مدخلات القرص بدل ثلاث، والانتظار بين فحص وآخر يُحسب من نهاية الفحص ويتضاعف ما دام القرص لا يهدأ (10 ثم 20 ثم 40 ثم 80 دقيقة، ثم ساعتان). / **Lighter whole-drive rescans:** at most two sets of a drive's entries instead of three, and the pause between walks counts from the end and doubles while the drive keeps changing (10, 20, 40, 80 minutes, then two hours).
- **حدّ لقراءة المجلدات الجديدة**: كل مجلد جديد (استيراد صور، فكّ أرشيف) كان يأخذ خيطاً خاصاً فوراً، فتبدأ آلاف الخيوط معاً. الآن 4 على الأكثر في وقت واحد و500 تنتظر دورها، وما زاد يصبح فحصاً واحداً للقرص. / **Folder-walk cap:** new folders are walked at most 4 at a time with up to 500 waiting; beyond that the drive is walked once instead of one thread per folder.
- **حارس الذاكرة**: يقيس البرنامج ذاكرته كل 30 ثانية، ويكتب في `log.txt` سطراً كل 10 دقائق وعند تجاوز 1 و2 و4 غيغابايت. إذا تجاوزت ثلاثة أضعاف الحجم المتوقع لفهرسك و1.5 غيغابايت معاً (مرة كل 30 دقيقة على الأكثر)، يوقف مراقبة الملفات ويترك التغييرات المنتظرة والذاكرة المؤقتة التي يمكن إعادة بنائها، ثم يعود إلى المراقبة بعد 30 دقيقة ويفحص الأقراص بهدوء؛ وإن خرجت قبل ذلك يفحصها التشغيل التالي. / **Memory guard:** samples memory every 30 s and logs it every 10 min and on passing 1, 2 and 4 GB. Above 3× the expected size for your index and 1.5 GB (at most once per 30 min) it stops watching, drops waiting changes and rebuildable caches, watches again after 30 min with a paced walk of each drive, and marks the index for a rescan at the next start.
- **الذاكرة في البحث داخل المستندات**: كلمة شائعة («في»، «a») في عشرات آلاف المستندات كانت تحجز لكل نتيجة حتى 50 سطراً، فتصل إلى غيغابايتات (وتنهار النسخة 32 بت). الآن تحتفظ أول 1,000 نتيجة فقط بسطورها، وتُقرأ سطور الباقي عند فتح «كل التطابقات» (F4) في الخلفية، ويتوقف البحث عند 100,000 نتيجة مع رسالة «تُعرض أول … نتيجة فقط». / **Memory in document search:** only the first 1,000 results keep their matching lines (the rest are read on a worker thread for All matches, F4), and a search lists at most 100,000 results, saying so.
- **سجل أوضح**: سطور الحفظ والنسخ والطوابير في `log.txt` مرة في الدقيقة على الأكثر لكل نوع، مع عدد ما لم يُكتب؛ الأخطاء تُكتب دائماً. / **A readable log:** index save, copy and queue lines at most once a minute per kind, with a count of those held back; failures are always written.
- **ذاكرة أقل وقت الراحة**: بعد دقيقتين بلا استعمال، وبعد انتهاء قراءة المستندات، يتخلّص البرنامج من الذاكرة المؤقتة (المصغّرات، أحجام المجلدات، المساحات الفارغة) ويعيدها إلى ويندوز، دون أن يمسّ الفهرس، فيبقى البحث التالي فورياً. / **Less memory when idle**: after two idle minutes, once documents are read, InstaFind drops its rebuildable caches and gives free memory back to Windows, without touching the index, so the next search stays instant.
- **الشكل الجديد لا يحجز ذاكرة وهو مخفي**: كانت عمليات WebView2 الست تبقى نحو 140 ميجابايت والنافذة مخفية بجانب الساعة. الآن تُعلَّق الصفحة عند الإخفاء، وبعد 10 دقائق مخفية بلا انقطاع يُغلق WebView2 كله فتنتهي عملياته (0 ميجابايت)؛ عند فتح النافذة تظهر فوراً كما الآن ثم تُحمَّل الصفحة من جديد بالبحث نفسه ومرشّحاته. لا يُحسب ذلك عطلاً ولا يفتح الشكل البسيط، ولا يحدث أثناء نقل نافذة حفظ/فتح ولا ونافذة حوار مفتوحة. نافذة البحث السريع (Ctrl+Alt+F) لا تتأثر. / **The new look holds no memory while hidden:** the hidden page is suspended at once, and after 10 minutes hidden without a break WebView2 is closed so its processes exit (they held about 140 MB); showing the window brings it at once and loads the page again with the same search and filters. Not counted as a failure, never the simple look, never during a Save/Open hand-off or under a dialog; the quick popup is unaffected.

### التحديث والتشغيل والخروج / Updates, start and exit
- **بعد التحديث تعمل نسخة واحدة دائماً، لا نسختان ولا صفر**: حتى 0.10.0 كانت النسخة الجديدة تنتظر القديمة دقيقة ثم قد تعمل بجانبها أو تخرج فلا يبقى شيء يعمل، وكان البرنامج القديم يُحذف قبل أن تثبت الجديدة أنها تعمل. الآن تراقب النسخة القديمة الجديدةَ 3 ثوانٍ: إن عملت خرجت القديمة، وإن انتهت الجديدة فوراً أُعيد البرنامج السابق وبقيت القديمة تعمل وقالت إن التحديث لم يُطبَّق. والنسخة الجديدة تنتظر القديمة 10 ثوانٍ على الأكثر ثم تُنهيها (بعد التأكد من أنها هي نفسها لا برنامج آخر أخذ رقمها)، ولا تحذف الملف القديم إلا بعد أن تبدأ فعلاً. الشيء نفسه عند نقل البرنامج إلى مجلده الدائم وعند إعادة التشغيل بعد تغيير اللغة. / **After an update exactly one copy runs, never two and never none:** the old copy watches the new one for 3 s and leaves only once it runs; if it ends at once the previous program is put back and the old copy keeps running and says so. The new copy waits at most 10 s for the old one (identified by process id and creation time), then ends it, and deletes the old file only after it has started. The same for the move to the permanent folder and the restart after a language change.
- **تجربة الإصدار الجديد**: إذا لم تكمل النسخة الجديدة بدء تشغيلها (النافذة، WebView2، تحميل الفهرس)، يعيد التشغيل التالي الإصدار السابق ويقول: «لم يعمل الإصدار الجديد من البرنامج، فأُعيد الإصدار السابق.». وإعادة الملفات تُجرَّب عدة مرات إن كان برنامج حماية يمسك الملف. / **Update trial:** a new version that never finishes starting is replaced by the previous one at the next start ("The new version of the program did not start, so the previous version was put back."); the renames are retried while security software holds the file.
- **الحفظ والخروج عند إطفاء ويندوز**: لم يكن البرنامج يجيب ويندوز عند إيقاف التشغيل أو تسجيل الخروج، فتضيع الإعدادات المتغيّرة منذ آخر إغلاق. الآن يحفظ الإعدادات والسجل ويمنح الفهرس انتظاراً قصيراً ثم يخرج. / **Saving and leaving when Windows shuts down:** at shutdown or sign-out the settings and history are saved, the index gets a short bounded wait, and the program leaves.
- **مؤقّت الخروج**: لكل طريقة خروج مهلة (الخروج العادي 10 ثوانٍ، وإعادة التشغيل بعد التحديث أو النقل أو تغيير اللغة 5 ثوانٍ، وإطفاء ويندوز 5 ثوانٍ)، وبعدها يُنهي البرنامج نفسه، فلا تعلق نافذة متجمدة ولا تنتظر النسخة الجديدة ولا يتأخر إطفاء الجهاز. الإعدادات تُكتب أولاً، والفهرس عبر ملف مؤقت، وما لم يُحفظ يُعلَّم للتشغيل التالي. والملف الذي كان يُقرأ لحظة الإنهاء لم يعد يُتخطّى إلى الأبد كأنه سبب انهيار. / **Exit watchdog:** every way of leaving has a deadline (exit 10 s, restart after an update, move or language change 5 s, Windows shutdown 5 s), after which the process ends itself; settings first, the index through a temp file, unsaved changes marked for the next start. A file being read at that moment is no longer skipped for good as if it had crashed the program.
- **«InstaFind» في مدير المهام**: مدير المهام وشريط المهام وتبويب «التفاصيل» في المستكشف كانت تعرض «bahth»؛ صار للملف وصف واسم منتج «InstaFind» (الشركة FastKiwi). اسم الملف يبقى `bahth.exe`. / **"InstaFind" in Task Manager**, the taskbar and Explorer's Details tab (version resource: FileDescription and ProductName InstaFind, company FastKiwi). The file name stays `bahth.exe`.
- **لا عرض لتحديث ذاتي مصيره الفشل**: نسخ Microsoft Store وChocolatey، والنسخ في مجلد لا يستطيع البرنامج الكتابة فيه (مثل Program Files)، كانت تُعرض عليها كل يوم تحديثات تفشل دائماً. الآن لا تُعرض، والتحقق اليدوي يقول كيف تُحدَّث هذه النسخة (`choco upgrade instafind`، أو المتجر، أو التنزيل من fastkiwi.app أو نقل البرنامج إلى مجلد خاص بك)، كما في نسخ winget وScoop. / **No self-update that can only fail:** Microsoft Store and Chocolatey copies, and copies in a folder the program cannot write to (Program Files), get no update offer; a manual check says how this copy is updated, as for winget and Scoop.
- **حدّ طلبات GitHub**: حين يرفض GitHub الطلبات مؤقتاً كان البرنامج يقول خطأً «لم ينشر GitHub بصمة SHA-256» ويعرض التحديث كل يوم. الآن: «تعذّر التحقق الآن، سنحاول لاحقاً.» ويعيد المحاولة بعد ساعة، ولا يُعرض شيء قبل التحقق، ولا يُثبَّت شيء أبداً دون بصمة. / **Behind GitHub's rate limit** the check says "Could not verify the update right now; we will try again later." and retries hourly, instead of wrongly saying no checksum was published; nothing is ever installed without a digest.
- **«التشغيل مع ويندوز» يبقى على النسخة المنقولة**: بعد نقل البرنامج إلى مكانه الدائم، تشغيل النسخة القديمة من «التنزيلات» (من اختصار أو تثبيت قديم) لم يعد يعيد التشغيل التلقائي إليها ما دامت النسخة المنقولة موجودة. / **Start with Windows stays on the moved copy:** an old Downloads or temp copy no longer points the Run entry back at itself while the installed copy exists.
- **اللغة في أول تشغيل**: تُحدَّد اللغة قبل أن يكتب البرنامج أي شيء في مجلد بياناته، فيبدأ تثبيت جديد على ويندوز بالإنجليزية بالواجهة الإنجليزية (كانت نسخ الاختبار قبل 1.0 تبدأ بالعربية؛ 0.10.0 لم يتأثر). / **Language at first start:** detected before anything is written to the data folder, so a new install on English Windows starts in English (1.0 test builds started in Arabic; 0.10.0 was not affected).
- **تحديثات موقّعة (minisign)**: صار المحدِّث قادراً على التحقق من توقيع كل تحديث (`bahth.exe.minisig` و`bahth-x86.exe.minisig` و`bahth-arm64.exe.minisig`) بمفاتيح مضمّنة في البرنامج، ويجب أن يذكر التوقيع رقم الإصدار نفسه («instafind v…»)، ويُفحص الملف أثناء تنزيله بجانب بصمة SHA-256. ما دامت قائمة المفاتيح فارغة يبقى التحقق ببصمة SHA-256 وحدها كما هو؛ وبعد إضافة المفاتيح يُرفض أي تحديث بلا توقيع صالح مع رابط للتنزيل اليدوي. في CI مهمة `sign` في بيئة `release` توقّع الملفات الثلاثة، وتُتخطّى بتحذير ما دام السر `MINISIGN_SECRET_KEY` غير موجود، والإصدار يُنشر كمسودة أولاً ولا يُعلَن إلا بعد التأكد من كل ملفاته. المفاتيح يصنعها المالك مرة واحدة بالسكربت `keys/ceremony.sh` في Codespace مؤقت. الخطوات: docs/specs/signing.md. / **Signed updates (minisign):** the updater can check a minisign signature `<exe>.minisig` for each update, made by a key compiled into the app and naming the release (`instafind <tag>`), streamed next to the SHA-256. While the key list is empty nothing changes (SHA-256 only); once keys are added, an update without a valid signature is refused. CI signs in a `sign` job in the `release` Environment (skipped with a warning until the secret exists) and publishes through a checked draft; `keys/ceremony.sh` makes the keys.

### الشكل الجديد / The new look
- **الشكل الجديد هو الشكل الوحيد**: يظهر لكل المستخدمين حيث يمكن تشغيله (فيه WebView2)، ومن كان اختار الشكل القديم يُنقل إليه مرة واحدة عند أول تشغيل. لم يعد هناك «العودة إلى الشكل القديم». إذا كان مكوّن WebView2 في ويندوز مفقوداً أو لا يعمل، يعمل البرنامج بالشكل البسيط في هذه المرة فقط ويقول السبب في سطر فوق النتائج، ثم يجرّب الشكل الجديد من جديد عند التشغيل التالي (وإن تعذّر مرة أخرى انتقل إلى البسيط بسرعة دون انتظار)، و«خيارات ← جرّب الشكل الجديد» يجرّبه فوراً. / **The new look is the only look** where it can run (WebView2); classic users are moved once, and "Back to the classic look" is gone. If WebView2 is missing or broken, the simple (classic) window is used for that run only, with a line saying why; the next start tries the new look again (falling back quickly if it still fails), and Options › Try the new look retries at once.
- **عرض المصغرات** (Ctrl+T أو زر الشبكة): النتائج مربعات بصورها المصغّرة كما في المستكشف، تُحمَّل للظاهر منها فقط، وتتنقل بينها بالأسهم وEnter وF2 وDelete والقائمة كالقائمة العادية. / **Thumbnail view** (Ctrl+T or the grid button): tiles with Explorer's thumbnails, loaded only for those in view, with the same keys and menu as rows.
- **«الإبلاغ عن مشكلة…»** في «الإعدادات ← المزيد» وفي «خيارات» (مجموعة الدعم) وقائمة أيقونة الساعة: نافذة صغيرة تكتب فيها ما حدث، وتعرض بالضبط ما سيُشارَك (الإصدار، نظام ويندوز، طريقة التثبيت، الشكل، حجم الفهرس وعدد المستندات، أحجام ملفات البيانات، الذاكرة الآن وأعلى ما بلغته، مدة التشغيل، الإعدادات المهمة، وآخر 80 سطراً من السجل)، مع زرّين: «نسخ» و«فتح صفحة الإبلاغ». لا يُرسل شيء إلا إذا ضغطت أنت، وأسماء الملفات والمسارات وعناوين البريد والشبكة واسم المستخدم والجهاز تُحذف من التقرير. / **Report a problem…** (Settings › More, Options, tray menu): shows exactly what would be shared (version, Windows, install kind, look, index and content index size, data-file sizes, memory now and peak, uptime, settings, last 80 log lines) with Copy and Open the report page; nothing is sent unless you press a button, and file names, paths, e-mail and IP addresses and account names are removed.
- **«خيارات أخرى»**: «الإعدادات ← المزيد ← فتح قائمة الخيارات…» يفتح قائمة «خيارات» كاملة: التحقق من التحديثات، الدعم والملاحظات، الملفات المكررة، أحجام المجلدات، استبدال النص، البحث من الهاتف، وإعدادات خانة البحث وAlt+Space. وعناصرها تعمل على الشكل الجديد: التصدير والملفات المكررة يأخذان نتائجه، وأحجام المجلدات مجلده، و«إخفاء بجانب الساعة» يخفيه، ورسائل الحالة والتحديث (ومنها نسبة التنزيل) تظهر فوقه. / **More options:** Settings › More › Open the Options menu… opens the full Options menu, and its items act on the new look (export, duplicates, folder sizes, hide); their messages, update offers and download progress show over it.
- **سحب ملف من النتائج** إلى المستكشف أو البريد أو المحادثة، و**أزرار البرامج** «تطبيق: …» فوق النتائج عندما يطابق البحث اسم برنامج، كما في الشكل القديم. وحجم النافذة ومكان تكبيرها يُحفظان. / Drag a result out, **App buttons** above the results, and the window's size and maximized state are kept, as in the classic window.
<!-- pending: new-look parity items still coming (G1 Ctrl+Alt+G in Save/Open dialogs, G2 popup suggestions, G3 Open with / Properties, G4 work area, G5 clear history and recent files, G6 hide balloon, G7 failed copy, G8 Store rating row, G9 update button, G10 message-box owners, G13 save on hide). Add one line each here when merged. -->

### نسخة Microsoft Store / Microsoft Store edition
- **نسخة Microsoft Store**: البرنامج نفسه في حزمة MSIX (x64 و32 بت وARM64) تُبنى بعد كل إصدار (`store.yml`) لتُرسل إلى المتجر. في نسخة المتجر: التحديثات من المتجر وحده (التحقق من التحديثات يعرض فتح صفحة البرنامج في المتجر)، والتشغيل مع ويندوز عبر مهمة بدء التشغيل وهو مطفأ حتى يشغّله المستخدم، ولا بحث من الهاتف، و«حول» يفتح تراخيص المكوّنات الخارجية (THIRD-PARTY-NOTICES.txt). / **Microsoft Store edition:** the same program in an MSIX bundle built after each release for submission; updates come from the Store (the update item offers the Store page), start with Windows goes through the package's startup task and is off until the user turns it on, no phone search, and About opens the third-party notices.
<!-- pending: the Store listing link, once the owner's submission is certified. -->

### البحث والعربية / Search and Arabic
- **«مسئول» و«مسؤول» و«مسوول» تجد بعضها** مباشرةً في أسماء الملفات، وكذلك «شئون»/«شؤون»/«شوون» و«رئيس»/«رءيس»/«ريس»؛ وما زالت الكتابة بلا همزة تعمل كما كانت («سوال» تجد «سؤال»، «مسايل» تجد «مسائل»، «بيية» تجد «بيئة»). الواو والياء تتبادلان فقط في مكان الهمزة، والاسم المكتوب كما كتبته يأتي أولاً، والأسماء تُعرض كما هي. داخل المستندات: البحث بالهمزة يجد الكتابات الأخرى. / **Hamza seat spellings meet:** «مسئول», «مسؤول» and «مسوول» (also شئون/شؤون/شوون, رئيس/رءيس/ريس) find each other in file names, and typing without the hamza still works; names spelled as typed rank first. In documents, a query with a hamza finds the other spellings.
- **الهمزة في آخر الكلمة وداخل المستندات**: «شاطئ» و«شاطيء» و«شيء» و«شئ» تجد بعضها، و«مسوول» و«ريس» و«شوون» تجد «مسئول» و«رئيس» و«شئون» داخل المستندات أيضاً. / **Word-final hamza and hamza inside documents**: شاطئ/شاطيء and شيء/شئ find each other, and مسوول, ريس, شوون also find مسئول, رئيس, شئون inside documents.
- **الكتابات الأخرى للهمزة تطابق كلمة كاملة فقط**: «رئيس» لم تعد تجد «إدريس» و«باريس» و«تدريس»، و«نهائي» لا تجد «نهاية» ولا تضعها أولاً، و«جزئي» لا تجد «جزيرة»، في الأسماء وداخل المستندات. الكتابة الأخرى (مثل «ريس» لـ«رئيس») تُطابق كلمةً كاملة فقط، ويجوز قبلها «ال» أو «و»، وما كتبته أنت يُطابق في أي مكان كما كان. والاستثناء («-رئيس») يستعمل الكلمة كما كتبتها فقط، فلا يخفي كل ملف فيه «ريس». / **Other hamza spellings match whole words only:** «رئيس» no longer finds إدريس/باريس/تدريس, «نهائي» no longer finds or ranks نهاية first, «جزئي» no longer finds جزيرة, in names and in documents. The other spelling (ريس for رئيس) matches only a whole word, optionally after ال or و; what you typed still matches anywhere. An exclusion («-رئيس») uses the word as typed only.
- **أرقام الهواتف والتواريخ في الأسماء العربية تظهر كما كُتبت**: في «فاتورة … بتاريخ 2026-11-12 … هاتف 0770-123-4567.txt» كان الهاتف يظهر 4567-123-0770 والتاريخ 12-11-2026. صارت الأرقام المتصلة بـ - / . : , تُعرض بترتيبها في النتائج والمسار والمعاينة وشريط الحالة والبحث السريع وصفحة الهاتف؛ الاسم نفسه لا يتغيّر (الفتح والنسخ وإعادة التسمية والبحث كما هي). / **Phone numbers and dates in Arabic names read as typed** (0770-123-4567, not 4567-123-0770) in results, paths, preview, status bar, quick popup and phone page; display only, the name itself is unchanged.
- **سطر الأوامر يصحّح مثل البرنامج**: `bahth -s hgjrvdv` (لغة لوحة المفاتيح الخطأ) تجد «التقرير»، و`-s ta2rer` (فرانكو) تجد «تقرير»، والأخطاء الإملائية كذلك، حين لا يجد البحث شيئاً كما كُتب؛ وشكل المخرجات لم يتغيّر. / **The command line falls back like the window** (wrong keyboard layout, Arabizi, typos) when the query as typed finds nothing; output unchanged.
- **`*` وحدها تعني كل الملفات في سطر الأوامر**: `bahth -s * --count` كانت تطبع 0 وتخرج برمز 1 لأن `*` بُحث عنها كحرف. الآن `*` و`*.*` و`ext:*` وحدها تعطي كل الملفات (مع `--scope` و`--all` والفلاتر الأخرى)، وبجانب كلمات لا تفلتر شيئاً كما كانت `*.*`. في النافذة يبقى المربع الذي فيه `*` وحدها مثل المربع الفارغ. / **A lone `*` means every file on the command line**: `bahth -s * --count` printed 0 and exited 1. `*`, `*.*` and `ext:*` alone now list every file (with `--scope`, `--all` and other filters); next to words they filter nothing. In the window a box with only `*` stays like an empty box.
- **تصحيح لغة لوحة المفاتيح مع المرشحات**: «`hgjrvdv size:>1mb`» يصبح «التقرير size:>1mb» بدل تحويل المرشح أيضاً إلى حروف عربية؛ كل المرشحات تبقى كما كُتبت. / The wrong-keyboard-layout fallback keeps every filter (`size:`, `dm:`, `regex:`…) as typed.
- **«تقريب:» و«جذر:» في البحث داخل المستندات** كانا يُتجاهلان بصمت؛ الآن تقول رسالة الحالة إنهما يعملان على أسماء الملفات فقط وإن الكلمات بُحث عنها كما كُتبت. / `fuzzy:` and `root:` in a document search now say they apply to file names only.
- **الأسماء التي تبدأ بنقطة** تُوجد الآن: «`.gitignore`» و«`.env`» و«`.ssh`» و«`.net framework`»؛ و«`ext:.pdf`» يعمل مثل «`ext:pdf`»، و«`*.*`» يعني كل الأنواع. / **Names that start with a dot** (`.gitignore`, `.env`, `.ssh`, `.net framework`) are found; `ext:.pdf` works like `ext:pdf`, and `*.*` means every type.
- **المدد مع المقارنة في `dm:`**: «`dm:>7d`» كان يُرفض، و«`dm:<=7d`» كان يطابق كل الملفات. الآن `>` و`>=` تعني بعد تلك اللحظة (أحدث)، و`<` و`<=` قبلها (أقدم)، و`dm:30d..7d` بين لحظتين؛ وهي مشروحة في README. / `dm:>7d` and `dm:<=7d` mean newer / older than 7 days ago, and `dm:30d..7d` works; documented in the README.
- **`dm:` والتوقيت الصيفي**: التاريخ يُحسب بتوقيت يومه نفسه ويُقرأ عند كل بحث، فيتفق مع عمود «تاريخ التعديل» حتى بعد تغيير الساعة أو المنطقة الزمنية. / **`dm:` and daylight saving:** each date uses its own day's offset, read at search time, so it agrees with the Date column.
- **كتابات أخرى لأسماء المرشحات العربية**: «كلمه:» و«تأريخ:» و«مُجلد:» تعمل مثل «كلمة:» و«تاريخ:» و«مجلد:». / Arabic filter prefixes accept the usual spelling variants (`كلمه:`, `تأريخ:`, `مُجلد:`).
- **البحث داخل مجلد اسمه فيه حروف مثل «İ»** (`C:\Users\İbrahim`) كان لا يجد شيئاً؛ المقارنة الآن حرفاً حرفاً. / Searching in a folder whose name has letters like `İ` works.
- **`regex:` ونهاية السطر**: «`regex:…$`» داخل المستندات يطابق نهاية السطور المنتهية بـ `\r\n` (أغلب ملفات النصوص في ويندوز). / `regex:…$` matches at the end of CRLF lines inside documents.
- **التحديد يبقى على ملفه** عند انتهاء «البحث داخل المستندات» وترتيب نتائجه، فلا يفتح Enter أو يحذف Delete ملفاً آخر. / **The selection stays on its file** when a document search ends and its rows are sorted.
- **ترتيب نتائج نافذة البحث الصغيرة**: بعد التطبيق أو صفحة الإعدادات المقصودة لم تعد تظهر ملفات لا علاقة لها جاءت من تخمين لوحة المفاتيح («المفكرة» ثم mathgl)، وصفحات الإعدادات تسبق الملفات التي تشبه البحث فقط. / Quick search: no unrelated keyboard-layout-guessed files after the app or settings page that was meant; Settings pages before files that only resemble the query.

### أمان / Security
- **لا تنزيل من السحابة دون إذنك**: كانت فهرسة المحتوى والبحث داخل المستندات والملفات المكررة والمصغرات تقرأ ملفات OneDrive (وغيره) الموجودة على السحابة فقط، فيُنزّلها ويندوز («InstaFind is downloading from OneDrive»). صار البرنامج يتعرّف عليها من خصائصها دون فتحها ويتخطّاها: أسماؤها تظهر في البحث كالعادة، ومحتواها يُقرأ حين تصبح على جهازك. لقراءتها رغم ذلك (وتنزيلها): «قراءة محتوى الملفات الموجودة على السحابة فقط (يُنزّلها من الإنترنت)» في الإعدادات، وهو مغلق افتراضياً. / **No cloud downloads behind your back:** content indexing, document search, duplicates and thumbnails no longer read cloud-only files (OneDrive Files On-Demand and other providers); names are still found, contents are read once the file is on the PC. Setting "Read contents of cloud-only files (downloads them)", off by default.
- **مصغّرة ملف كان على السحابة** تظهر بعد تنزيله (خلال نصف دقيقة)، لا بعد إعادة تشغيل البرنامج. / A **thumbnail of a file that was cloud-only** appears once it is downloaded (within half a minute), not after a restart.
- **ملفات مصمَّمة أو تالفة لا توقف البرنامج**: ملف PDF بحلقة لا تنتهي (نموذج يرسم نفسه، أو صفحة هي أصل نفسها)، وملف xlsx صغير يعلن عن عدد هائل من النصوص، وملف xls يكرّر نصاً طويلاً في آلاف الخلايا (ملف 2 ميجابايت أخذ نحو 9 غيغابايت) كانت تُغلق البرنامج أو تستهلك ذاكرته؛ صارت تُفحص وتُرفض أو تُقرأ بالطريقة الآمنة. / **Crafted or damaged files no longer bring the program down:** PDF loops (a form drawing itself, a page tree that is its own parent), an xlsx declaring trillions of shared strings, and an xls repeating one long string in many cells (a 2 MB file took ~9 GB) are checked first and refused or read the safe way.
- **حماية من تكرار الانهيار**: إذا أُغلق البرنامج فجأة أثناء قراءة ملف، يتخطّى ذلك الملف في التشغيل التالي (ويذكره في السجل) إلى أن يتغيّر أو يصدر إصدار جديد، بدل أن ينهار عند كل تشغيل. / **Crash-loop guard:** a file the app died reading is skipped at the next start (and logged) until it changes or a new version tries it once more.
- **ملفات مصمَّمة تُبطئ الفهرسة لساعات** (RTF بمئات آلاف الخطوط، رسائل eml بفواصل أو عناوين مشوّهة، PDF بنماذج متشابكة أو آلاف الصور) صارت تُقرأ بسرعة، ولكل ملف حدّ زمني: ما يتجاوزه يُحفظ «بلا نص» ولا يُعاد في كل تشغيل. ولكل ملف حدّ واحد لما يُفكّ ضغطه، يشمل ما بداخله من مستندات وأجزاء. / **Files crafted to take hours** are now fast, and every file has a time budget (one that runs over is stored as having no text) and one decompression budget, members and their parts included.
- **البحث من الهاتف يعمل في شبكة المنزل فقط**: بعد فتح البرنامج (ومنه التشغيل المخفي مع ويندوز) لا يبدأ وحده إلا على الشبكة التي فعّلته عليها وما دام ويندوز يعدّها شبكة خاصة، لا في شبكة مقهى أو فندق أو جامعة. يتوقف حين يتغيّر الاتصال، وتشرح نافذته السبب مع زر «السماح بهذه الشبكة»، وينتقل إلى عنوان الحاسوب الجديد إن تغيّر. أثناء عمله يظهر «البحث من الهاتف يعمل» في تلميح أيقونة الساعة. / **Phone search stays at home:** it starts by itself only on the network it was turned on at, while Windows marks it private; it stops on any other network, says why, offers "Allow this network", follows address changes, and the tray tip shows "Phone search is on".
- **الرمز السري للبحث من الهاتف**: الروابط ذات الرمز الخاطئ (`?t=`) لم تعد تُحسب ولا تقفل شيئاً، وحدّ محاولات الرمز السري صار لكل جهاز مع سقف صغير لكل الأجهزة معاً؛ وإيقاف البحث من الهاتف يغيّر الرمز السري والرابط، فلا يعمل أي هاتف سجّل الدخول سابقاً. / **Phone search PIN:** wrong `?t=` links are not counted, the PIN limit is per device with a small global ceiling, and turning it off makes a new PIN and link, ending old sessions.
- **ملفات DLL مزروعة بجانب البرنامج**: مكتبات النظام (مثل `version.dll`) تُحمَّل من System32 فقط، عند بدء التشغيل وأثناء العمل، لا من مجلد البرنامج («التنزيلات» مثلاً)، وفحص البناء يرفض أي نسخة تفقد هذه الحماية؛ وقراءة النص من الصور (OCR) لا تبدأ إلا إذا كانت مكوّناتها مسجّلة في ويندوز. / **DLL planting:** system DLLs load from System32 only (`/DEPENDENTLOADFLAG:0x800` and `SetDefaultDllDirectories`), never from the exe's folder, checked in CI; OCR starts only when its WinRT classes are registered.
- **برامج ويندوز بمسارها الكامل**: «فتح المجلد» يشغّل مستكشف الملفات من مجلد ويندوز، وأدوات البحث السريع («إدارة الأجهزة»، «لوحة التحكم»، «محرر التسجيل»…) من System32 أو مجلد ويندوز فقط، فلا يعمل برنامج بالاسم نفسه وُضع في «التنزيلات». / **Windows programs by full path:** explorer.exe and the quick search's classic tools start from System32 or the Windows folder only.
- **حمايات ويندوز ضد الاستغلال مثبّتة صراحةً**: ASLR (بنطاق 64 بت) وDEP وحماية تدفق التحكم (Control Flow Guard) في ملفات exe المنشورة، وفحص البناء يتحقق منها. / **Exploit mitigations:** /DYNAMICBASE, /NXCOMPAT, /HIGHENTROPYVA (64-bit) and Control Flow Guard in the published exes, checked in CI.

### أمان البيانات / Data safety
- **الملفات المكررة لا تحذف آخر نسخة**: قبل النقل إلى سلة المحذوفات يُفحص القرص من جديد، ولا يُحذف ملف إلا إذا بقيت منه نسخة غير محددة ما زالت موجودة ومطابقة. ما لا يتحقق فيه ذلك يُترك ويُذكر مع السبب. تغيير طريقة المقارنة يمسح القائمة، وفي «نفس الاسم والحجم» يقول التأكيد صراحةً إن المحتوى لم يُقارَن. / **Duplicate finder never deletes the last copy:** the disk is re-checked right before the Recycle Bin; a file goes only while an unselected copy still exists and still matches; skipped files are listed with the reason.
- **الملف نفسه بمسارين ليس نسخة مكررة**: الروابط الصلبة والوصلات (junctions) وأقراص `subst` تُعرف برقم الملف على القرص، فتُعرض «نفس الملف» ولا تُحذف كأنها نسخة. / **The same file under two paths is not a duplicate** (hard links, junctions, `subst` drives).
- **الاستبدال بالتعبير النمطي لا يحذف نصاً بصمت**: «‎$1em» تعني المجموعة 1 ثم «em»، والمجموعة غير الموجودة تُرفض برسالة واضحة، والمعاينة تعرض ما سيُكتب بالضبط. / **Regex replace no longer deletes text silently:** `$1em` is group 1 then "em"; a missing group is refused.
- **الاستبدال لا يفسد ملفات بترميزات أخرى**: الملف الذي ليس UTF-8 يُعامل على أنه Windows-1256 فقط إذا قُرئ نصاً عربياً؛ ملفات Shift-JIS وGBK وBig5 وغيرها تُترك وتظهر «ترميز غير معروف — لم يُغيَّر». / **Replace no longer corrupts files in other encodings:** other non-UTF-8 files are skipped as "unknown encoding — not changed".
- **تصدير النتائج (CSV) يفتح في أعمدة في Excel** حتى حيث فاصل القوائم في ويندوز «;». / **CSV export opens in columns in Excel** with Windows' list separator.

### خصوصية / Privacy
- **تقرير العطل يخفي اسم المستخدم كاملاً**: «C:\Users\الاسم الأول الأخير» يصبح «<user>» بكل كلماته، وكذلك «Documents and Settings» ومسارات الشبكة، واسم الحساب أينما ورد في النص. / **Crash reports hide the whole user name** (spaces included), Documents and Settings, UNC paths, and the account name anywhere in the text.
- **«مسح السجل والملفات الأخيرة»** (كان «مسح عمليات البحث الأخيرة») يمسح أيضاً الملفات المفتوحة مؤخراً وسجل ما فُتح من البحث السريع، ومنه عمليات البحث على الويب التي تحفظ النص المكتوب. البحوث المحفوظة والمفضلة تبقى. (في الشكل البسيط؛ الشكل الجديد قادم.) / **"Clear history and recent files"** also forgets recently opened files and quick-search launches (web searches held the typed text); saved searches and pins stay (classic window; the new look is pending).

### إصلاحات / Fixes
- **بعد تثبيت حزمة اللغة العربية لقراءة الصور** تُعاد قراءة ملفات PDF الممسوحة والصور التي لم يُعثر فيها على نص، مرة واحدة. / After the **Arabic OCR language pack** is installed, scanned PDFs and images found empty are read once more.
- **قراءة الصور (OCR) في الخلفية** صورة واحدة في كل مرة، وتتوقف أثناء الكتابة أو البحث. / **Background OCR** runs one image at a time and waits while you type or search.
- **فهرس المحتوى ينسى الملفات المحذوفة** (مرة يومياً على الأكثر)، مع حمايات: لا شيء أثناء تحميل الفهرس أو إعادة بنائه، ولا لقرص مفصول أو مجلد شبكة غير متصل، وربع الفهرس على الأكثر في كل مرة. / The **content index forgets deleted files** (at most daily), never while loading or rescanning, never for an unplugged drive or offline share, at most a quarter per pass.
- **النصوص الأطول من 80 ميجابايت** تُحفظ مقصوصة بدل إعادة قراءتها في كل تشغيل، وملف .xls ليس جدولاً حقيقياً لم يعد يُخرج نصه مرتين. / **Texts over 80 MB** are stored cut to the limit; a non-workbook **.xls** no longer yields its text twice.
- **أيقونة شريط المهام تعود بعد إعادة تشغيل Explorer** (بلونها وتلميحها وحالة إظهارها). / The tray icon comes back after Explorer restarts.
- **«إنهاء» تطبيق متجر مصغّر لا يغلق كل تطبيقات المتجر**: يُرفض الأمر برسالة توضح السبب بدل إنهاء ApplicationFrameHost. / "End" on a minimized Store app no longer ends ApplicationFrameHost (and every Store app with it).
- **«إفراغ سلة المحذوفات» لا يجمّد البرنامج**: يجري في الخلفية بعد التأكيد، وتظهر رسالة إذا فشل. / Emptying the Recycle Bin runs in the background, so the app no longer freezes.
- **المفضلة تحفظ وسائط صفحات الإعدادات القديمة**: تثبيت «الأجهزة والطابعات» لم يعد يفتح لوحة التحكم العادية. / Pinned classic settings pages keep their arguments.
- **إعداد Alt+Space لا يضيع بصمت** إذا سبقك برنامج آخر إلى الاختصار: يبقى مفعّلاً، وتقول القائمة إن برنامجاً آخر يستخدمه، ويعيد البرنامج المحاولة حتى يتحرر. / The Alt+Space setting is kept when another program holds the key; it is retried until free.
- **النسخ الفاشل لا يُعلن نجاحاً** في النافذة البسيطة ونافذة البحث الصغيرة: يعيد المحاولة، وإن فشل تبقى النافذة مفتوحة مع رسالة قصيرة. / A failed clipboard copy is retried and then reported (classic window and quick popup).
- **Ctrl+Alt+G لا يعمل إلا في نوافذ الحفظ والفتح الحقيقية** (خانة اسم الملف وعرض الملفات معاً)، لا في «البحث عن» أو «الطباعة». / Ctrl+Alt+G only acts on real Open/Save dialogs.
- **عرض المصغرات لا يكدّس أيقونات الأنواع**: أول 64 نوعاً فقط لها أيقونة خاصة، والبقية تشترك في أيقونة الملف العامة. / The thumbnail view keeps at most 64 file-type icons.
- **النافذة البسيطة تبقى فوق شريط المهام**: تُفتح في وسط مساحة العمل وتعود إلى داخل الشاشة عند إظهارها بعد تغيّر الشاشة. / The classic window opens centred in the work area and is pulled back on screen when shown.
- **لا مربعات منقّطة حول المسارات** («LRI» و«PDI»)؛ تبقى المسارات من اليسار إلى اليمين بعلامات لا يرسمها ويندوز. / No dotted "LRI"/"PDI" boxes around paths.
- **عنوان عمود «النص المطابق» يظهر كاملاً**: الأعمدة الافتراضية تتوزّع على عرض القائمة ما دمت لم تغيّر عرضها بنفسك. / The default column widths fit the list.
- **«أسماء الملفات جاهزة»**: بعد اكتمال قائمة الأسماء وأثناء قراءة المستندات يقول شريط الحالة ونافذة البحث الصغيرة «أسماء الملفات جاهزة · تُقرأ محتويات المستندات: 36,000 من 432,000» بدل «جارٍ الفهرسة…». / Once names are indexed the status reads "File names ready · reading inside documents: N of M".
- **الدقائق الأولى لا تبدو متوقفة**: أثناء بناء الفهرس لأول مرة تشرح القائمة الفارغة ما يحدث، والبحث داخل المستندات الذي لم يجد شيئاً بعد يقول كم قرأ، و«لا نتائج» صار جملة واضحة فوق أزرار الاقتراحات. / The empty list explains the first index build, a document search still reading says how far it got, and "no results" has a plain sentence.
- **رسالة إخفاء النافذة البسيطة أوضح**: «بحث سريع ما زال يعمل. افتحه بـ Ctrl+Alt+F. للإغلاق نهائياً: خيارات ← خروج من البرنامج.» / The classic window's one-time balloon says how to open InstaFind again and how to exit.
- **لا سؤال عند أول تشغيل عن خانة البحث فوق الساعة**: صارت خياراً مطفأً تُشغَّل من «الإعدادات ← البحث من أي مكان»، ومن اختارها سابقاً تبقى له. / **No first-run question about the box above the clock**: it is an option, off until turned on in Settings → Search from anywhere; an earlier choice is kept.
- **قائمة «خيارات» مرتّبة في مجموعات** تفصل بينها خطوط، و«إخفاء بجانب الساعة» (كان «إخفاء إلى شريط المهام») و«خروج من البرنامج» في آخرها؛ وفي النافذة البسيطة انتقل «تصدير النتائج» إلى قائمة النقر بالزر الأيمن، وفيها أيضاً «فتح باستخدام…» و«خصائص». / The Options menu is grouped with separators, "Hide next to the clock" and Exit last; in the classic window "Export results", "Open with…" and "Properties" are in the result right-click menu.
- **مصطلحات وعبارات موحّدة**: «جارٍ» في كل رسائل الانتظار، و«خانة البحث» في قائمتَي الأيقونة والخانة العائمة، و«خيارات…» في قائمة الأيقونة، وعبارة خانة البحث أبسط («اكتب اسم الملف أو جزءاً منه… مثال: تقرير .pdf») وتُقرأ من اليمين في الواجهة العربية. / Consistent Arabic terms; a simpler search hint that reads right to left in Arabic.
- **مفاتيح نافذة البحث الصغيرة ظاهرة** تحت خانة البحث، وخانة البحث العائمة لها اسم يقرؤه قارئ الشاشة. / The quick popup shows its keys; the floating box has a name screen readers announce.
- **جواب تحويل العملات**: «سعر تقريبي · التاريخ · Enter للنسخ»، وسطر مصدر الأسعار لم يعد جواباً تقف عليه الأسهم. / Currency answers say "approximate rate · date · Enter copies"; the arrow keys pass over the rates source line.

### الإصدار والصيانة / Release and maintenance
- **SHA256SUMS.txt مع كل إصدار**: ملف البصمات الرسمي مرفق بالإصدار بجانب الملفات الثلاثة، لتتحقق أن ما نزّلته سليم. / **SHA256SUMS.txt** is attached to every release next to the three exes, the official checksums.
- اختبار الشكل الجديد في CI يغطي المستخدم الجديد (بلا إعدادات)، ونقل من اختار الشكل القديم في 0.10.0 ونسخ الاختبار قبل 1.0 مرة واحدة، وغياب أي طريق إلى الشكل القديم، والرجوع إلى الشكل البسيط عند تعطّل WebView2 لمرة واحدة ثم عودة الشكل الجديد في التشغيل التالي (`BAHTH_WEBUI_FAIL=1`)؛ واختبار «المستخدم الحقيقي» يطلب النافذة البسيطة بمتغيّر خاص بالاختبارات (`BAHTH_CLASSIC=1`). / CI covers the new user, the one-time move, no way back and the one-run fallback (`BAHTH_WEBUI_FAIL=1`); the classic-window test uses `BAHTH_CLASSIC=1`.
- اختبار الدخان يتحقق من اسم «InstaFind» في معلومات كل ملف exe منشور، وحزمة المتجر تُبنى فقط لإصدار منشور أو يدوياً. / The smoke test checks the InstaFind version resource of every published exe; the Store package is built only for a published release or by hand.
<!-- pending: hard Windows suites (mem-test, torture, ui-stress, 2-hour soak: R1, R2, R9) if they gate 1.0. -->

## 0.10.0 — 2026-11-11

### الرخصة
- InstaFind أصبح برنامجاً مملوكاً (مجانياً)؛ إشعارات الأطراف الثالثة دون تغيير. / **Licence:** InstaFind is now proprietary (free); third-party notices unchanged.

### أمان
- **calamine 0.38**: إصلاح ثغرة RUSTSEC-2026-0436 في قراءة ملفات Excel القديمة (.xls و.xlsb)، وهي ملفات يفتحها الفهرس تلقائياً.

### إصلاحات
- **امتداد الملف في الأسماء العربية** يظهر كاملاً في نهاية الاسم («.xlsx» على يسار «تقرير المبيعات ٢٠٢٦») بدل «xlsx.٢٠٢٦ …»، في قائمة النتائج وفي الشكل الجديد (النتائج والمعاينة والملفات الأخيرة)، ويبقى تظليل الكلمة المطابقة كما هو.
- **نافذة البحث الصغيرة** ترسم الياء الأخيرة بنقطتيها: «الطبي» لا «الطبى» (كانت نقاط الحروف تحت السطر تُقصّ).
- **المسارات في الواجهة العربية** معزولة باتجاه من اليسار لليمين، فتبقى «\» في مكانها الصحيح بين المجلدات وداخل الجمل العربية.
- **تنبيه عند غياب حزمة اللغة العربية لقراءة الصور (OCR)**: البحث داخل المستندات يقول إن النص العربي في الصور لا يمكن قراءته، بدل تخطّيه بصمت.
- **خانة البحث الرئيسية** تُظهر عبارتها التوضيحية وأمثلتها حتى وهي مفعّلة.
- رسالة أول تشغيل تذكر «InstaFind» باسمه، وكلمة «الإنجليزية» موحّدة في الواجهة والموقع والوثائق.

### جديد
- **جرّب الشكل الجديد**: «خيارات ← جرّب الشكل الجديد» يفتح الواجهة الجديدة فوراً دون إعادة تشغيل ويتذكّرها في المرات القادمة، و«الإعدادات ← العودة إلى الشكل القديم» يرجعك إلى الشكل المعتاد متى شئت. الشكل القديم يبقى الافتراضي، والخيار يظهر فقط حيث يتوفر WebView2.
- **الدعم**: «خيارات ← الدعم» يفتح البريد على support@fastkiwi.app أو صفحة الدعم على fastkiwi.app (بالعربية في الواجهة العربية)، ونافذة «حول» تذكرهما.
- **تقييم في المتجر (نسخة Microsoft Store فقط)**: بعد فتح 5 ملفات من النتائج يظهر مرة واحدة سطر هادئ تحت النتائج «قيّم InstaFind في المتجر» أو «ليس الآن»، ولا يعود بعد أي جواب، و«خيارات ← التقييم في المتجر» يبقى متاحاً؛ العدّ على جهازك فقط.
- **إسناد أسعار الصرف**: سطر «Rates By Exchange Rate API» مع رابط موقعهم تحت كل جواب لتحويل العملات، كما تطلب شروطهم المجانية.

### الوثائق
- الخصوصية في README والموقع تذكر تنزيل أسعار الصرف (مرة كل 12 ساعة تقريباً)، و«أكثر من مليون ملف» بدل «ملايين الملفات».

### الصيانة
- فحص cargo-deny في CI (الثغرات والتراخيص والمصادر لأهداف ويندوز، `deny.toml`) وفحص zizmor لملفات GitHub Actions.
- ملفات exe المنشورة تُبنى دون ذاكرة بناء مخزّنة (cache) لمنع تسميمها، و`persist-credentials: false` في كل خطوة checkout لا تحتاج الرفع، وصلاحيات packaging.yml على مستوى المهمة.
- اختبار سجل الأعطال (crash_log) لم يعد متقطّع النجاح: اختباراته التي تُطلق أخطاء تعمل واحداً تلو الآخر.

## 0.9.1 — 2026-11-03

### أمان
- **لا تحديث بدون بصمة**: إذا لم ينشر GitHub بصمة SHA-256 لملف التحديث، يرفض البرنامج التحديث ويعرض رابط التنزيل اليدوي، بدل التثبيت دون تحقق.
- **نسخ winget و Scoop** لا تحدّث نفسها بعد الآن؛ تعرض أمر التحديث المناسب (`winget upgrade Sarmad.InstaFind` أو `scoop update instafind`).

## 0.9.0 — 2026-11-02

### بحث البرامج
- **أكثر من 45 برنامجاً ولعبة جديدة بأسمائها العربية** وتهجئات أكثر للموجودة، ومنها الكتابة العراقية بالكاف: «كوكل كروم»، «تلكرام»، «وتساب»، «بوربوينت»، «فوتشوب»، «بوبجي»، «فيفا»، «جاتا».
- **حرف غلط واحد مقبول** في نطق اسم البرنامج: «ديسكود» تجد Discord و«نتفليس» Netflix، وبداية الاسم تكفي: «سبوت»، «تيليج»، «مايكروسوفت ور».
- **البرنامج يتعلّم من اختيارك**: البرنامج الذي تفتحه بعد كتابة حروف معيّنة يظهر أولاً في المرة الجاية.
- **أزرار البرامج في النافذة الرئيسية**: «تطبيق: …» فوق النتائج عندما يطابق البحث اسم برنامج بقوة.

### إجابات فورية
- **تحويل العملات** لـ 24 عملة بالعربي والإنجليزي: «100 دولار»، «100 usd to iqd»، «50 يورو بالدينار»، «١٠٠ ريال سعودي». الدولار يتحول للدينار العراقي تلقائياً. الأسعار تتحدث في الخلفية كل 12 ساعة من open.er-api.com، ومع جدول تقريبي مضمّن إذا ما في إنترنت.

### نافذة البحث
- **الملفات المفضلة والأخيرة** تظهر عند فتح البحث فارغاً (في النافذة الصغيرة والرئيسية). اسم «المثبّتة» صار «المفضلة».
- **اقتراحات عندما لا توجد نتائج**: «هل تقصد…؟»، البحث داخل المحتوى، كل الأقراص، كل الأنواع، أو الويب.
- **ترتيب أذكى**: «notepda» تُظهر Notepad قبل الملفات، و«2+2» تعطي الجواب بدون ملفات لا علاقة لها.

### السرعة
- **فهرسة محتوى الملفات في اليوم الأول أسرع بمرة ونصف إلى 3 مرات**، وتبدأ بسطح المكتب والمستندات والتنزيلات، وتكتب على القرص أقل بكثير.

### جودة
- اختبار «مستخدم حقيقي» على ويندوز يجب أن ينجح قبل أي نشر.

## 0.8.2 — 2026-11-02

### إصلاحات
- **توقف البرنامج فجأة على بعض الأجهزة** عند الضغط المتكرر على خانة البحث العائمة أو عند الإغلاق، فتختفي الخانة والأيقونة بجانب الساعة: كُبّرت ذاكرة المكدس للنافذة من 1 إلى 8 ميغابايت، ومُنع فتح نافذة البحث الصغيرة وإغلاقها داخل بعضهما.
- **كل توقف مفاجئ يُسجَّل الآن** في تقرير الخطأ، حتى الذي لا يأتي من البرنامج نفسه (مثل امتلاء المكدس)، فيسألك البرنامج عند الفتح التالي إن كنت تريد إرساله.
- البحث من الهاتف لا يتوقف عند ضغط زر الإغلاق، ويُسجَّل في ملف السجل ما فعله زر الإغلاق.
- **Enter في نافذة البحث الصغيرة يفتح أفضل نتيجة** حتى لو كان المؤشر واقفاً فوق نتيجة أخرى: كان اتساع النافذة تحت مؤشر ثابت يحدد الصف الذي تحته.

## 0.8.1 — 2026-11-02

### إصلاحات
- **زر الإغلاق (X) لا يغلق البرنامج بعد الآن**: يخفيه إلى الأيقونة بجانب الساعة، وتبقى خانة البحث العائمة وCtrl+Alt+F تعمل. للخروج فعلاً: «خروج» من قائمة الأيقونة أو «خيارات ← خروج من البرنامج».
- **التشغيل مع ويندوز** يُفعَّل من جديد مرة واحدة، حتى لو أوقفه «مدير المهام ← تطبيقات بدء التشغيل»، ويظهر في «خيارات» كما هو فعلاً. وتقدر تطفيه بعدها.
- **إصلاح تعطل بعد مدة من التشغيل** مع خانة البحث العائمة (المشكلة #10): كان البرنامج يستهلك موارد الرسم في ويندوز حتى تنفد، فيتوقف وتبقى الخانة على الشاشة بلا عمل.
- عند الخروج تختفي الخانة العائمة فوراً.

### بحث البرامج
- **البرامج تُوجد بكتابتها بالعربي كما تُنطق**، لأي برنامج وليس فقط لقائمة الأسماء المعروفة: «كوكل بلي» أو «جوجل بلاي» تجد Google Play Games، و«نتفلكس» Netflix، و«كاب كات» CapCut.
- **البرامج المثبتة حديثاً تظهر في البحث** دون إعادة تشغيل البرنامج (القائمة تتجدد في الخلفية).

## 0.8.0 — 2026-11-02

### جديد: البحث من أي مكان
- **خانة بحث صغيرة فوق الساعة**: شفافة وتكاد لا تُرى حتى تقرّب المؤشر. مكانها يسار عمود إشعارات ويندوز فلا تتغطى، وتنعكس في ويندوز العربي، وتختفي مع الألعاب والفيديو بملء الشاشة وتتبع شريط المهام المخفي تلقائياً، وتُسحب لأي مكان. ثلاثة أوضاع: تختفي تقريباً (الافتراضي)، مخفية حتى تقترب من الزاوية، أو مطفأة. وفي أول تشغيل يسألك البرنامج إن كنت تريدها.
- **نافذة بحث صغيرة** بـ Ctrl+Alt+F أو من أيقونة الساعة: الملفات، و**البرامج المثبتة** بأسمائها العربية (كروم، وورد، واتساب… نحو 200 برنامج)، و**صفحات إعدادات ويندوز** (نحو 190 صفحة: واي فاي، بلوتوث، الشاشة، التحديثات…)، و**التبديل إلى النوافذ المفتوحة**.
- **إجابات فورية**: حاسبة تقبل الأرقام العربية، وتحويل الوحدات، و**التاريخ الهجري بتقويم أم القرى** وتحويل التواريخ.
- **اختصارات المواقع**: يوتيوب وجوجل وويكيبيديا والخرائط والترجمة.
- **أوامر النظام**: إيقاف التشغيل وإعادة التشغيل والسكون والقفل وتسجيل الخروج وإفراغ السلة وإنهاء برنامج معلّق، **وكلها تطلب تأكيداً**.
- **Tab** لإجراءات النتيجة (تشغيل كمسؤول، نسخ المسار، فتح المجلد، التثبيت في المفضلة)، والبرنامج يقدّم ما تفتحه كثيراً.
- كتابة «مشكلة» أو «اقتراح» تفتح الإبلاغ مباشرة، وزر «النتيجة غلط؟» حين لا يجد البحث شيئاً.
- **أيقونة بلون واحد بجانب الساعة** تتبع الوضع الداكن والفاتح.
- كل الجديد يُطفأ من «خيارات ← البحث من أي مكان»، ويمكن إرجاع Ctrl+Alt+F لفتح النافذة الكاملة كما كان.

### جودة
- سرعة البحث وزمن التشغيل مثل 0.6.1 أو أسرع (قيست على ويندوز حقيقي بنفس الفهرس)، والنافذة الصغيرة تفتح في نحو 12 ملّي ثانية.
- لقطات وفحوص تلقائية للواجهة الجديدة على ويندوز حقيقي بالعربية والإنجليزية والوضعين الداكن والفاتح.

## 0.7.0 — 2026-11-01

### جديد
- **نسخة أصلية لأجهزة ARM64** (مثل Snapdragon): `bahth-arm64.exe`، أسرع وأقل استهلاكاً للبطارية من تشغيل نسخة 64 بت بالمحاكاة.
- **نسخة لويندوز 32 بت**: `bahth-x86.exe`.
- نسخة 64 بت تبقى `bahth.exe`. كل نسخة تحدّث نفسها بنسختها، وScoop وwinget يختاران النسخة المناسبة للجهاز.
- **«خيارات ← أرسل ملاحظة»**: «أبلغ عن مشكلة» و«اقترح ميزة جديدة» و«اسأل أو ناقش مع المستخدمين»، كل منها يفتح صفحة GitHub جاهزة فيها رقم الإصدار ونظام ويندوز ونوع المعالج.
- **تقارير الأخطاء باختيارك**: بعد خطأ داخلي يسألك البرنامج مرة واحدة عند التشغيل التالي، و«نعم» تفتح صفحة GitHub فيها التقرير لتراجعه وترسله بنفسك (اسم المستخدم يُخفى). لا يُرسل شيء تلقائياً ولا تُجمع بيانات استخدام.

### جودة
- كل نسخة (64 بت و32 بت وARM64) تُشغَّل فعلياً على ويندوز بمعالجها قبل النشر، ومنها جهاز ARM حقيقي.

## 0.6.1 — 2026-11-01

### إصلاحات
- **الخط تحت الجزء المطابق في الواجهة العربية كان في المكان الخطأ**: في أسماء الملفات العربية التي لها امتداد لاتيني (مثل «تقرير المبيعات 2026.xlsx») كان الخط يُرسم مزاحاً إلى اليسار بعرض الامتداد، فيقع تحت كلمة أخرى. أصبح يقع تحت الكلمة التي بحثت عنها تماماً، وتم التحقق من ذلك بلقطات شاشة على ويندوز حقيقي.

## 0.6.0 — 2026-11-01

### جديد
- **إخفاء ملفات النظام** (مفعّل افتراضياً): مجلد Windows وProgramData وسلة المحذوفات وملفات الصفحات ومجلدات المؤقت والذاكرة المخبأة داخل AppData لا تظهر في النتائج، فتبقى نتائجك نظيفة. يُطفأ من «خيارات ← إخفاء ملفات النظام» ويعمل فوراً دون إعادة فهرسة، أو اكتب `نظام:` لبحث واحد. ملفاتك وProgram Files وملفات Outlook وOffice واختصارات قائمة ابدأ تبقى ظاهرة. في سطر الأوامر: `--all`.
- **واجهة أنظف**: أزرار بأحجام متناسقة وأيقونات صغيرة (ويندوز 10 و11)، مسافات موحّدة، خلفية بيضاء بلا مربعات رمادية، زر «بحث» مميَّز، و«البحث في» بدل مربع النطاق الرمادي. تتكيف مع النوافذ الضيقة.

### ثبات وجودة
- مراجعة شاملة للبرنامج كله أصلحت نحو 45 خللاً قد يسبب إغلاقاً أو تعليقاً أو فقدان بيانات، منها:
  - ملفات Excel وODS و7z وPDF المعطوبة أو المصممة بخبث كانت قد تستهلك الذاكرة كلها وتغلق البرنامج.
  - ملف فهرس معطوب كان قد يمنع البرنامج من الفتح في كل مرة.
  - فشل بحث واحد كان قد يوقف كل عمليات البحث اللاحقة حتى إعادة التشغيل.
  - تحديث مقطوع كان قد يترك الجهاز بلا نسخة من البرنامج؛ أصبحت النسخة القديمة تُستعاد.
  - الاستبدال في الملفات: لا يضيع الملف الأصلي إن فشلت الكتابة، والتراجع يعيد كل شيء.
  - حماية أقوى لخادم البحث من الهاتف.
  - النصوص والخطوط كانت تُكبَّر مرتين على الشاشات عالية الدقة (150% فأكثر).
  - اختيار المجلد `C:\Docs` كان يشمل `C:\Docs2` أيضاً.
- الاختبارات الآلية من 262 إلى 364، منها اختبارات عشوائية لكل نوع ملف تغذيه ببيانات تالفة وتتأكد أنه لا ينهار.
- كل إصدار يُشغَّل الآن فعلياً على ويندوز قبل نشره: يفهرس القرص كله (أكثر من مليون ملف) ويبحث بالعربية والإيموجي والمسارات الطويلة، ولا يُنشر إن ظهر أي خطأ.

## 0.5.1 — 2026-10-30

### إصلاحات
- بعض المستندات (رسائل البريد وصفحات HTML التي فيها رموز مثل `&` بجانب علامات اقتباس ‘ ’ أو خطوط ─) كان استخراج نصها يفشل فلا تظهر في البحث داخل الملفات، وقد يكون هذا سبب إغلاق البرنامج المفاجئ النادر في 0.5.0. أصبح يُقرأ نصها كاملاً.

## 0.5.0 — 2026-10-30

### جديد
- **واجهة إنجليزية** باسم **InstaFind**: تتبع لغة ويندوز في التثبيت الجديد، وتبقى عربية لمن كان يستخدم البرنامج، وتُغيَّر من «خيارات ← Language / اللغة». الواجهة الإنجليزية من اليسار لليمين.
- **التشغيل مع ويندوز** في الخلفية (في شريط المهام)، مفعّل افتراضياً ويُطفأ من «خيارات».
- **تحديث الفهرس تلقائياً** عند إضافة الملفات أو حذفها أو إعادة تسميتها، دون إعادة فهرسة يدوية.
- **فهرس دائم لمحتوى المستندات**: يُحفظ نص كل مستند مضغوطاً فيصبح البحث داخل الملفات المتكرر شبه فوري.
- **البحث داخل الصور وملفات PDF الممسوحة ضوئياً** بمحرك التعرف على النصوص المدمج في ويندوز 10 وما بعده.
- **البحث داخل الملفات المضغوطة** zip و7z وtar وtar.gz، مع حماية من الأرشيفات الضخمة.
- **بحث يتحمّل الأخطاء الإملائية** تلقائياً عند عدم وجود نتائج، وكتابة **الفرانكو** (`3abd` ← عبد)، و`جذر:` للبحث بجذر الكلمة.
- **القفز إلى مجلد النتيجة في نوافذ الحفظ والفتح** بأي برنامج: Ctrl+Alt+G ثم اختر نتيجة.
- **كشف الملفات المكررة** (بالحجم ثم بصمة المحتوى) مع نقل المختار إلى سلة المحذوفات.
- **أحجام المجلدات**: عمود الحجم للمجلدات، ونافذة تعرض ما يستهلك مساحة القرص.
- **عرض المصغرات** للنتائج (Ctrl+T).
- **استبدال نص في عدة ملفات نصية** مع معاينة ونسخة احتياطية وزر تراجع.
- **البحث من الهاتف** عبر الشبكة المنزلية: مطفأ افتراضياً، محمي برمز دخول، وللقراءة فقط.
- **إضافة Flow Launcher** وأمثلة PowerShell وAutoHotkey في مجلد `integrations/`.

### ثبات
- أي خطأ داخلي في نافذة البرنامج يُسجَّل في log.txt وcrash.txt مع تفاصيله، ولا يُغلق البرنامج.

## 0.4.1 — 2026-10-29

### إصلاحات
- التحديث التلقائي لم يعد يتوقف عندما يحدّ GitHub عدد الطلبات (60 طلباً في الساعة لكل جهاز): يلجأ البرنامج إلى صفحة أحدث إصدار مباشرة وينزّل منها.
- رسالة أوضح عند رفض الطلب، وتُسجَّل نتيجة كل تحقق من التحديثات وكل خطأ في log.txt.

## 0.4.0 — 2026-10-29

### جديد
- البحث بالحجم: `حجم:>10م`، `size:1mb..5mb`، `حجم:كبير`، `size:empty`.
- البحث بتاريخ التعديل: `تاريخ:اليوم`، `تاريخ:أمس`، `dm:thisweek`، `dm:2024-05`، `dm:>2024-01-01`، `dm:7d`.
- التعبيرات المنتظمة `regex:` ومطابقة الكلمة كاملة `كلمة:` أو `ww:`.
- البحث بالتقارب: `"عقد إيجار"~5` يجد الكلمتين ضمن خمس كلمات من بعضهما.
- كل التطابقات داخل المستند: عددها يظهر في عمود النص المطابق، وF4 (أو القائمة) يعرض كل الأسطر المطابقة مع أرقامها.
- سطر الأوامر: `bahth.exe -s "..."` مع `--max` و`--sort` و`--scope` و`--count` و`--csv` و`--json`.
- تنبيه في شريط الحالة إذا تعذر فهم جزء من صيغة البحث، مثل `حجم:>abc`.

### إصلاحات
- بعد التحديث التلقائي يغلق الإصدار القديم فوراً، ولا يخرج الإصدار الجديد بصمت إذا تأخر القديم في الإغلاق، ويُسجَّل ذلك في log.txt.

## 0.3.1 — 2026-10-29

### تحسينات داخلية (لا تغيير في طريقة الاستخدام)
- تقسيم ملف الواجهة (أكثر من 2000 سطر) إلى 11 وحدة صغيرة واضحة، مما يسرّع إضافة الميزات وإصلاح الأخطاء.
- تبسيط وحدات البحث والفهرسة واستخراج النصوص وإزالة التكرار فيها.
- 98 اختباراً آلياً بعد أن كانت 78، تشمل حالات خاصة في صيغة البحث والمسارات العربية وملفات الإعدادات.
- فحص آلي على GitHub مع كل تعديل: clippy دون تحذيرات، واختبارات على لينكس وويندوز، وبناء البرنامج.
- حفظ ملف الإعدادات صار آمناً عند انقطاع الكهرباء (ملف مؤقت ثم استبدال).

## 0.3.0 — 2026-10-29

### جديد
- **تحديث تلقائي**: يتحقق البرنامج مرة يومياً من وجود إصدار أحدث على GitHub ويعرض تثبيته، ثم يستبدل نفسه ويعيد التشغيل. ويمكن التحقق يدوياً من قائمة «خيارات».
- اختصار عام **Ctrl+Alt+F** يُظهر البرنامج من أي مكان (ويُخفيه إذا كان ظاهراً).
- خط ملوّن تحت الجزء المطابق من اسم كل نتيجة.
- ترتيب النتائج بالنقر على عنوان العمود، مع سهم يبيّن الاتجاه، ويُحفظ آخر ترتيب.
- زر «السجل» (Ctrl+H): آخر 20 عملية بحث، وحفظ البحث الحالي باسم، وتشغيل البحوث المحفوظة بنقرة.
- الملفات التي تفتحها كثيراً أو مؤخراً تظهر أولاً.
- F2 لإعادة التسمية وDelete للنقل إلى سلة المحذوفات، مع تحديث الفهرس فوراً.
- سحب النتيجة وإفلاتها في أي برنامج، وCtrl+C ينسخ الملف نفسه (Ctrl+Shift+C للمسار).
- البحث داخل رسائل البريد (eml وmsg من Outlook) وكتب epub.
- تصحيح العربية المعكوسة في ملفات PDF («يف» تصبح «في») ليعمل البحث داخلها.

### إصلاحات
- إعادة التسمية لا تستبدل ملفاً موجوداً بالاسم نفسه.
- حفظ الفهرس من أكثر من مكان في وقت واحد لم يعد يُتلف الملف، والتعديلات تُحفظ قبل الإغلاق.
- إعادة التسمية ترفض الأسماء التي لا يقبلها ويندوز (نقطة أو مسافة في النهاية، CON، NUL…).
- التحديث التلقائي يتحقق من بصمة SHA-256 للملف المنزّل إن كانت منشورة، ويعيد الإصدار القديم إذا فشل الاستبدال.

## 0.2.2 — 2026-10-29

### إصلاحات
- ظهور النافذة أسرع بكثير: كان كل تغيير في حجم أي زر يعيد ترتيب النافذة كلها من جديد بشكل متداخل، فيستغرق ظهورها نحو 7 ثوانٍ على بعض الأجهزة.
- البحث عن خط الواجهة يفحص الخطوط المطلوبة مباشرة بدل استعراض كل خطوط الجهاز مرتين.
- نافذتا «اختيار مجلد» و«حفظ النتائج» تُجهَّزان عند أول استخدام لا عند التشغيل.
- النافذة المكبّرة تُفتح مكبّرة مباشرة، ويُحسب ارتفاع شريط الحالة بعد ضبط خطه.
- سجل التشغيل يقيس كل مرحلة منذ بدء العملية حتى أول دورة للواجهة، ويُكتب خارج خيط الواجهة.

## 0.2.1 — 2026-10-29

### إصلاحات
- لم تعد النافذة تتجمّد («لا يستجيب») بعد التشغيل مع فهرس كبير: البحث بالاسم يعمل الآن في الخلفية، والكتابة السريعة تُجمع في بحث واحد.
- تحميل الفهرس وبناؤه بأولوية أقل من الواجهة.
- لم يعد البحث داخل المستندات يتوقف إذا انتهى بحث بالاسم قديم في الخلفية.
- يُلغى تحديد الصف عند تغيّر النتائج، فلا يفتح Enter ملفاً غير الذي اخترته.
- `عقد | ext:pdf تقرير` صار يعني «عقد» و«تقرير» معاً، لا «عقد أو تقرير».
- العبارة بين علامتي تنصيص تُطابق كما كُتبت دون حذف «ال».
- في البحث داخل المستندات، الكلمة المستبعدة (`!قديم`) تُفحص في النص فقط لا في اسم الملف.
- تصحيح لوحة المفاتيح يحافظ على فلتر النوع مثل `.pdf` في أي موضع.
- تصدير CSV آمن: الخلايا التي تبدأ بـ `=` أو `+` أو `-` أو `@` لا تُقرأ كمعادلات في Excel.
- النقر على «تحديث الفهرس» أثناء التحميل يعرض رسالة بدل أن يُتجاهل.
- تشغيل البرنامج مرتين بسرعة لا يفتح نسختين.
- حجم النافذة المحفوظ يُضبط ليناسب الشاشة الحالية.
- سجل أزمنة التشغيل في `%LOCALAPPDATA%\BahthSari\log.txt` للمساعدة في تشخيص البطء.

## 0.2.0 — 2026-10-29

### جديد
- أيقونة جديدة للبرنامج.
- قائمة نتائج بنمط مستكشف ويندوز مع أيقونات أنواع الملفات، ومربع بحث أكبر.
- فلتر أنواع جاهز: مستندات، صور، صوت، فيديو، مضغوطات، برامج، مجلدات.
- تصحيح الكتابة بلوحة المفاتيح الخطأ: `hgjrvdv` تعرض نتائج «التقرير».
- تجاهل «ال» وأخواتها: «التقرير» تجد «تقرير.pdf».
- توحيد أشكال العرض العربية (من ملفات PDF) والحروف الفارسية ک ی ە.
- عوامل البحث: `صورة|فيديو` (أو)، `!قديم` أو `-قديم` (استبعاد)، `مجلد:` (المجلدات فقط)، `ext:jpg;png`.
- تصدير النتائج إلى CSV يفتحه Excel بالعربية، ونسخ مسارات كل النتائج.
- اختصارات لوحة المفاتيح: السهم للأسفل، Ctrl+Enter، Ctrl+C، Ctrl+F، Esc.
- فتح فوري، ونسخة واحدة، وأيقونة في شريط المهام (Esc يخفي البرنامج).
- تواريخ نسبية («اليوم»، «أمس»، «قبل 5 دقائق»)، وخيار الأرقام العربية، وتذكّر حجم النافذة والأعمدة.
- استثناء node_modules و.git ومجلدات مشابهة من الفهرسة.

## 0.1.0

- الإصدار الأول: فهرسة الأقراص، بحث عربي موحّد بالاسم، البحث داخل المستندات، واجهة عربية من اليمين لليسار.

</div>
