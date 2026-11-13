//! Windows settings pages (`ms-settings:` URIs) with Arabic and English words,
//! plus the classic tools (Control Panel applets, `.msc` consoles) that the
//! shell opens by name.

use crate::i18n::Lang;

/// One settings page.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// `ms-settings:…`, or a command the shell opens (`devmgmt.msc`, `control`).
    pub uri: &'static str,
    pub title_ar: &'static str,
    pub title_en: &'static str,
    /// Section path shown under the title, per language.
    pub section_ar: &'static str,
    pub section_en: &'static str,
    /// Extra words in both languages ("wifi", "واي فاي", "انترنت").
    pub keywords: &'static [&'static str],
    /// Lowest Windows build that has this page (0 = all).
    pub min_build: u32,
    /// Highest Windows build that has this page (0 = no limit), for
    /// Windows 10 pages that Windows 11 dropped.
    pub max_build: u32,
}

impl Page {
    /// The title in the interface language (a translated one translates
    /// the English title). Matching uses `title_ar`, `title_en` and the
    /// keywords as they are.
    pub fn title(&self, english: bool) -> &'static str {
        Lang::from_english(english).pick(self.title_ar, self.title_en)
    }

    /// The section in the interface language.
    pub fn section(&self, english: bool) -> &'static str {
        Lang::from_english(english).pick(self.section_ar, self.section_en)
    }

    /// Whether the page exists on Windows build `build` (0 = unknown: yes).
    pub fn available(&self, build: u32) -> bool {
        build == 0 || (build >= self.min_build && (self.max_build == 0 || build <= self.max_build))
    }

    /// A `ms-settings:` page rather than a classic tool.
    pub fn is_settings_uri(&self) -> bool {
        self.uri.starts_with("ms-settings:")
    }

    const fn until(mut self, max_build: u32) -> Page {
        self.max_build = max_build;
        self
    }
}

/// First Windows 11 build.
pub const WIN11: u32 = 22000;
/// Last Windows 10 builds are 1904x; anything below Windows 11.
const WIN10_LAST: u32 = WIN11 - 1;

type Section = (&'static str, &'static str);

const SYSTEM: Section = ("النظام", "System");
const DEVICES: Section = ("البلوتوث والأجهزة", "Bluetooth & devices");
const NETWORK: Section = ("الشبكة والإنترنت", "Network & internet");
const PERSONAL: Section = ("التخصيص", "Personalization");
const APPS: Section = ("التطبيقات", "Apps");
const ACCOUNTS: Section = ("الحسابات", "Accounts");
const TIME: Section = ("الوقت واللغة", "Time & language");
const GAMING: Section = ("الألعاب", "Gaming");
const ACCESS: Section = ("إمكانية الوصول", "Accessibility");
const PRIVACY: Section = ("الخصوصية والأمان", "Privacy & security");
const UPDATE: Section = ("تحديث Windows", "Windows Update");
const SEARCH: Section = ("البحث", "Search");
const TOOLS: Section = ("أدوات Windows", "Windows tools");

const fn p(
    uri: &'static str,
    title_ar: &'static str,
    title_en: &'static str,
    section: Section,
    keywords: &'static [&'static str],
    min_build: u32,
) -> Page {
    Page { uri, title_ar, title_en, section_ar: section.0, section_en: section.1, keywords, min_build, max_build: 0 }
}

#[rustfmt::skip]
pub static PAGES: &[Page] = &[
    // System.
    p("ms-settings:display", "الشاشة", "Display", SYSTEM, &["display", "screen", "monitor", "resolution", "brightness", "scale", "orientation", "rotate", "شاشة", "شاشه", "الدقة", "دقة الشاشة", "السطوع", "الإضاءة", "تكبير", "حجم النص", "تدوير الشاشة", "شاشة ثانية"], 0),
    p("ms-settings:nightlight", "الإضاءة الليلية", "Night light", SYSTEM, &["night light", "blue light", "eye", "warm", "الوضع الليلي", "ضوء أزرق", "حماية العين", "اضاءة ليلية"], 0),
    p("ms-settings:display-advanced", "إعدادات العرض المتقدمة", "Advanced display", SYSTEM, &["refresh rate", "hz", "hertz", "معدل التحديث", "هرتز", "معلومات الشاشة"], 0),
    p("ms-settings:display-advancedgraphics", "الرسومات", "Graphics", SYSTEM, &["graphics", "gpu", "graphics card", "hardware accelerated gpu scheduling", "كرت الشاشة", "كارت الشاشة", "معالج الرسومات", "الرسوميات"], 17093),
    p("ms-settings:sound", "الصوت", "Sound", SYSTEM, &["sound", "audio", "volume", "speakers", "headphones", "microphone", "output", "input", "صوت", "مستوى الصوت", "السماعات", "سماعة", "المايك", "الميكروفون", "مكبر الصوت"], 0),
    p("ms-settings:sound-devices", "إدارة أجهزة الصوت", "Manage sound devices", SYSTEM, &["sound devices", "audio devices", "أجهزة الصوت", "سماعات", "ميكروفون"], 17063),
    p("ms-settings:apps-volume", "مستوى صوت التطبيقات", "Volume mixer", SYSTEM, &["volume mixer", "app volume", "mixer", "خالط الصوت", "صوت البرامج", "صوت التطبيقات"], 17063),
    p("ms-settings:notifications", "الإشعارات", "Notifications", SYSTEM, &["notifications", "alerts", "action center", "اشعارات", "التنبيهات", "مركز الإشعارات", "تنبيهات"], 0),
    p("ms-settings:quiethours", "مساعد التركيز / عدم الإزعاج", "Focus assist / Do not disturb", SYSTEM, &["focus assist", "do not disturb", "quiet hours", "focus", "dnd", "عدم الإزعاج", "التركيز", "الساعات الهادئة", "وضع التركيز"], 17074),
    p("ms-settings:powersleep", "الطاقة والسكون", "Power & sleep", SYSTEM, &["power", "sleep", "screen timeout", "turn off screen", "hibernate", "power mode", "الطاقة", "السكون", "النوم", "إيقاف الشاشة", "إطفاء الشاشة", "وضع الطاقة", "الإسبات"], 0),
    p("ms-settings:batterysaver", "البطارية", "Battery", SYSTEM, &["battery", "battery saver", "charge", "بطارية", "توفير البطارية", "موفر البطارية", "الشحن"], 0),
    p("ms-settings:batterysaver-usagedetails", "استخدام البطارية", "Battery usage", SYSTEM, &["battery usage", "battery use", "استهلاك البطارية", "استخدام البطارية حسب التطبيق"], 0),
    p("ms-settings:storagesense", "التخزين", "Storage", SYSTEM, &["storage", "disk space", "free space", "disk", "drive", "temporary files", "clean", "المساحة", "مساحة القرص", "التخزين", "القرص", "الملفات المؤقتة", "تنظيف", "تفريغ مساحة", "الهارد"], 0),
    p("ms-settings:storagepolicies", "استشعار التخزين", "Storage Sense", SYSTEM, &["storage sense", "auto clean", "recycle bin", "تنظيف تلقائي", "سلة المحذوفات", "مستشعر التخزين"], 0),
    p("ms-settings:savelocations", "مكان حفظ المحتوى الجديد", "Where new content is saved", SYSTEM, &["save locations", "default save location", "مكان الحفظ", "موقع الحفظ الافتراضي"], 0),
    p("ms-settings:disksandvolumes", "الأقراص ووحدات التخزين", "Disks & volumes", SYSTEM, &["disks", "volumes", "partition", "format", "الأقراص", "الأقسام", "تقسيم القرص", "تهيئة", "فورمات"], WIN11),
    p("ms-settings:multitasking", "تعدد المهام", "Multitasking", SYSTEM, &["multitasking", "snap", "snap layouts", "alt tab", "virtual desktops", "تعدد المهام", "تقسيم الشاشة", "سطح المكتب الافتراضي", "تجاور النوافذ"], 0),
    p("ms-settings:tabletmode", "وضع الكمبيوتر اللوحي", "Tablet mode", SYSTEM, &["tablet mode", "tablet", "الوضع اللوحي", "تابلت"], 0).until(WIN10_LAST),
    p("ms-settings:project", "العرض على هذا الكمبيوتر", "Projecting to this PC", SYSTEM, &["projecting", "miracast", "wireless display", "العرض اللاسلكي", "بث الشاشة", "ميراكاست"], 0),
    p("ms-settings:crossdevice", "التجارب المشتركة / المشاركة القريبة", "Shared experiences / Nearby sharing", SYSTEM, &["nearby sharing", "shared experiences", "share", "المشاركة القريبة", "مشاركة الملفات", "التجارب المشتركة"], 0),
    p("ms-settings:clipboard", "الحافظة", "Clipboard", SYSTEM, &["clipboard", "clipboard history", "copy paste", "win v", "حافظة", "سجل الحافظة", "النسخ واللصق", "نسخ ولصق"], 17666),
    p("ms-settings:remotedesktop", "سطح المكتب البعيد", "Remote Desktop", SYSTEM, &["remote desktop", "rdp", "remote", "سطح المكتب البعيد", "تحكم عن بعد", "الوصول عن بعد"], 17063),
    p("ms-settings:about", "حول", "About", SYSTEM, &["about", "pc name", "rename pc", "specs", "system info", "ram", "processor", "windows version", "device specifications", "حول الجهاز", "معلومات الجهاز", "مواصفات الجهاز", "اسم الكمبيوتر", "إعادة تسمية الكمبيوتر", "الرام", "المعالج", "إصدار ويندوز", "نسخة الويندوز"], 0),
    p("ms-settings:deviceencryption", "تشفير الجهاز", "Device encryption", PRIVACY, &["device encryption", "encryption", "bitlocker", "التشفير", "بت لوكر"], 0),
    p("ms-settings:activation", "التنشيط", "Activation", SYSTEM, &["activation", "activate windows", "product key", "license", "التفعيل", "تفعيل ويندوز", "مفتاح المنتج", "الترخيص", "تنشيط"], 0),
    p("ms-settings:troubleshoot", "استكشاف الأخطاء وإصلاحها", "Troubleshoot", SYSTEM, &["troubleshoot", "troubleshooter", "fix", "repair", "problems", "حل المشاكل", "إصلاح", "مستكشف الأخطاء", "مشاكل", "اصلاح المشاكل"], 0),
    p("ms-settings:recovery", "الاسترداد", "Recovery", SYSTEM, &["recovery", "reset this pc", "reset", "factory reset", "go back", "advanced startup", "safe mode", "الاستعادة", "إعادة تعيين", "فرمتة", "ضبط المصنع", "إعادة ضبط", "الوضع الآمن", "بدء التشغيل المتقدم", "الرجوع للإصدار السابق"], 0),
    p("ms-settings:developers", "للمطورين", "For developers", SYSTEM, &["developer mode", "developers", "dev mode", "وضع المطور", "المطورين"], 0),
    p("ms-settings:personalization-lighting", "الإضاءة الديناميكية", "Dynamic Lighting", PERSONAL, &["dynamic lighting", "rgb", "lighting", "إضاءة الكيبورد", "إضاءة"], 22631),
    // Bluetooth & devices.
    p("ms-settings:bluetooth", "البلوتوث", "Bluetooth", DEVICES, &["bluetooth", "pair", "headset", "airpods", "wireless", "بلوتوث", "بلوتووث", "بلوتث", "إقران", "اقتران", "سماعة بلوتوث", "اضافة جهاز"], 0),
    p("ms-settings:connecteddevices", "الأجهزة", "Devices", DEVICES, &["devices", "connected devices", "add device", "الأجهزة المتصلة", "إضافة جهاز"], 0),
    p("ms-settings:printers", "الطابعات والماسحات الضوئية", "Printers & scanners", DEVICES, &["printer", "printers", "scanner", "print", "طابعة", "الطابعة", "طباعة", "سكانر", "الماسح الضوئي", "اضافة طابعة"], 0),
    p("ms-settings:mousetouchpad", "الماوس", "Mouse", DEVICES, &["mouse", "cursor speed", "scroll", "primary button", "left handed", "ماوس", "الفأرة", "سرعة الماوس", "التمرير", "زر الماوس"], 0),
    p("ms-settings:devices-touchpad", "لوحة اللمس", "Touchpad", DEVICES, &["touchpad", "trackpad", "gestures", "تاتش باد", "لوحة اللمس", "الإيماءات", "الماوس اللمسي"], 0),
    p("ms-settings:pen", "القلم وWindows Ink", "Pen & Windows Ink", DEVICES, &["pen", "stylus", "windows ink", "handwriting", "القلم", "قلم", "الكتابة اليدوية"], 0),
    p("ms-settings:autoplay", "التشغيل التلقائي", "AutoPlay", DEVICES, &["autoplay", "auto play", "usb autorun", "التشغيل التلقائي", "الفلاشة"], 0),
    p("ms-settings:usb", "USB", "USB", DEVICES, &["usb", "يو اس بي", "منفذ usb", "الفلاشة"], 0),
    p("ms-settings:camera", "الكاميرات", "Cameras", DEVICES, &["camera", "webcam", "كاميرا", "الكاميرا", "كاميرا الويب"], WIN11),
    p("ms-settings:mobile-devices", "الأجهزة المحمولة / ربط الهاتف", "Mobile devices / Phone Link", DEVICES, &["phone", "phone link", "mobile", "android", "iphone", "الهاتف", "الجوال", "الموبايل", "ربط الهاتف", "هاتفك"], 0),
    p("ms-settings:typing", "الكتابة", "Typing", TIME, &["typing", "autocorrect", "spell check", "suggestions", "touch keyboard", "التصحيح التلقائي", "التدقيق الإملائي", "الاقتراحات", "لوحة المفاتيح التي تعمل باللمس", "الكتابة"], 0),
    p("ms-settings:wheel", "العجلة", "Wheel", DEVICES, &["wheel", "surface dial", "dial"], 0),
    // Network & internet.
    p("ms-settings:network-status", "حالة الشبكة", "Network status", NETWORK, &["network", "internet", "connection", "connected", "network reset", "شبكة", "الشبكة", "الانترنت", "انترنت", "إنترنت", "الاتصال", "إعادة تعيين الشبكة", "النت"], 0),
    p("ms-settings:network-wifi", "Wi-Fi", "Wi-Fi", NETWORK, &["wifi", "wi fi", "wireless", "wlan", "واي فاي", "وايفاي", "واي فاى", "الوايرلس", "لاسلكي", "شبكة لاسلكية", "انترنت", "الشبكة", "النت"], 0),
    p("ms-settings:network-wifisettings", "إدارة الشبكات المعروفة", "Manage known networks", NETWORK, &["known networks", "forget network", "saved networks", "wifi password", "الشبكات المحفوظة", "نسيان الشبكة", "شبكات واي فاي", "باسورد الواي فاي"], 0),
    p("ms-settings:network-ethernet", "Ethernet", "Ethernet", NETWORK, &["ethernet", "lan", "cable", "wired", "ip address", "dns", "ايثرنت", "إيثرنت", "كابل", "سلكي", "عنوان ip"], 0),
    p("ms-settings:network-vpn", "VPN", "VPN", NETWORK, &["vpn", "في بي ان", "شبكة افتراضية خاصة", "كسر الحجب"], 0),
    p("ms-settings:network-mobilehotspot", "نقطة اتصال محمولة", "Mobile hotspot", NETWORK, &["mobile hotspot", "hotspot", "tethering", "share internet", "نقطة اتصال", "نقطه اتصال", "هوت سبوت", "مشاركة الانترنت", "مشاركة النت"], 0),
    p("ms-settings:network-airplanemode", "وضع الطيران", "Airplane mode", NETWORK, &["airplane mode", "flight mode", "aeroplane", "وضع الطائرة", "وضع الطيران", "الطيران"], 0),
    p("ms-settings:network-proxy", "الوكيل", "Proxy", NETWORK, &["proxy", "بروكسي", "الوكيل", "خادم وكيل"], 0),
    p("ms-settings:network-dialup", "الطلب الهاتفي", "Dial-up", NETWORK, &["dial-up", "dialup", "modem", "pppoe", "broadband", "مودم", "الاتصال الهاتفي"], 0),
    p("ms-settings:network-cellular", "خلوي", "Cellular", NETWORK, &["cellular", "mobile data", "sim", "lte", "5g", "بيانات الجوال", "شريحة", "خلوي"], 0),
    p("ms-settings:datausage", "استخدام البيانات", "Data usage", NETWORK, &["data usage", "data limit", "metered", "استهلاك البيانات", "استهلاك النت", "حد البيانات", "اتصال محدود"], 0),
    p("ms-settings:network-advancedsettings", "إعدادات الشبكة المتقدمة", "Advanced network settings", NETWORK, &["network adapters", "adapter", "network reset", "محولات الشبكة", "إعادة تعيين الشبكة", "كرت الشبكة"], WIN11),
    // Personalization.
    p("ms-settings:personalization", "التخصيص", "Personalization", PERSONAL, &["personalization", "personalize", "customize", "تخصيص", "مظهر", "شكل ويندوز"], 0),
    p("ms-settings:personalization-background", "الخلفية", "Background", PERSONAL, &["background", "wallpaper", "desktop background", "slideshow", "خلفية", "خلفية سطح المكتب", "صورة الخلفية", "الخلفيات", "تغيير الخلفية"], 0),
    p("ms-settings:personalization-colors", "الألوان", "Colors", PERSONAL, &["colors", "colours", "dark mode", "light mode", "dark theme", "accent color", "transparency", "الوضع الداكن", "الوضع المظلم", "الوضع الليلي", "داكن", "دارك مود", "الوضع الفاتح", "لون التمييز", "الشفافية", "ألوان", "اسود"], 0),
    p("ms-settings:themes", "النُّسُق", "Themes", PERSONAL, &["themes", "theme", "desktop icons", "سمة", "السمات", "ثيم", "ثيمات", "أيقونات سطح المكتب", "النسق"], 0),
    p("ms-settings:lockscreen", "شاشة التأمين", "Lock screen", PERSONAL, &["lock screen", "lockscreen", "screen saver", "screensaver", "شاشة القفل", "قفل الشاشة", "شاشة التوقف", "حافظة الشاشة"], 0),
    p("ms-settings:personalization-start", "البدء", "Start", PERSONAL, &["start menu", "start", "recently added", "قائمة ابدأ", "ابدأ", "قائمة البدء", "الستارت"], 0),
    p("ms-settings:personalization-start-places", "المجلدات في البدء", "Folders on Start", PERSONAL, &["start folders", "folders next to power", "مجلدات قائمة ابدأ"], 0),
    p("ms-settings:taskbar", "شريط المهام", "Taskbar", PERSONAL, &["taskbar", "task bar", "system tray", "tray icons", "taskbar alignment", "شريط المهام", "التاسك بار", "الشريط السفلي", "أيقونات الشريط", "محاذاة شريط المهام"], 0),
    p("ms-settings:fonts", "الخطوط", "Fonts", PERSONAL, &["fonts", "font", "typeface", "install font", "خطوط", "الخط", "تثبيت خط", "خط عربي"], 17083),
    p("ms-settings:personalization-textinput", "إدخال النص", "Text input", PERSONAL, &["text input", "touch keyboard theme", "emoji panel", "ime", "لوحة الرموز التعبيرية", "إدخال النص", "ايموجي"], WIN11),
    // Apps.
    p("ms-settings:appsfeatures", "التطبيقات المثبتة", "Installed apps", APPS, &["apps", "installed apps", "apps & features", "uninstall", "remove program", "programs", "التطبيقات والميزات", "البرامج المثبتة", "إلغاء التثبيت", "الغاء تثبيت", "حذف برنامج", "إزالة برنامج", "حذف تطبيق", "البرامج", "إزالة التثبيت"], 0),
    p("ms-settings:optionalfeatures", "الميزات الاختيارية", "Optional features", SYSTEM, &["optional features", "add feature", "features on demand", "الميزات الاختيارية", "اضافة ميزة"], 0),
    p("ms-settings:defaultapps", "التطبيقات الافتراضية", "Default apps", APPS, &["default apps", "default browser", "default programs", "open with", "file associations", "التطبيقات الافتراضية", "المتصفح الافتراضي", "البرامج الافتراضية", "فتح باستخدام", "البرنامج الافتراضي", "تغيير المتصفح"], 0),
    p("ms-settings:maps", "الخرائط دون اتصال", "Offline maps", APPS, &["offline maps", "maps", "download maps", "الخرائط", "خرائط بدون انترنت"], 0),
    p("ms-settings:appsforwebsites", "تطبيقات لمواقع الويب", "Apps for websites", APPS, &["apps for websites", "open links in app", "روابط المواقع", "تطبيقات المواقع"], 0),
    p("ms-settings:videoplayback", "تشغيل الفيديو", "Video playback", APPS, &["video playback", "hdr video", "video", "تشغيل الفيديو", "الفيديو"], 0),
    p("ms-settings:startupapps", "تطبيقات بدء التشغيل", "Startup apps", APPS, &["startup", "startup apps", "startup programs", "autostart", "boot", "برامج بدء التشغيل", "بدء التشغيل", "البرامج التي تبدأ مع الويندوز", "التشغيل التلقائي للبرامج", "تسريع الاقلاع"], 17017),
    // Accounts.
    p("ms-settings:yourinfo", "معلوماتك", "Your info", ACCOUNTS, &["account", "your info", "microsoft account", "profile picture", "الحساب", "حسابي", "حساب مايكروسوفت", "صورة الحساب", "معلومات الحساب"], 0),
    p("ms-settings:emailandaccounts", "البريد الإلكتروني والحسابات", "Email & accounts", ACCOUNTS, &["email accounts", "accounts used by other apps", "add account", "البريد الالكتروني", "الايميل", "حسابات البريد", "إضافة حساب"], 0),
    p("ms-settings:signinoptions", "خيارات تسجيل الدخول", "Sign-in options", ACCOUNTS, &["sign-in options", "sign in", "password", "change password", "pin", "windows hello", "login", "تسجيل الدخول", "كلمة المرور", "كلمة السر", "الباسورد", "تغيير كلمة المرور", "رمز pin", "رقم سري", "قفل الجهاز", "بصمة الوجه", "البصمة"], 0),
    p("ms-settings:signinoptions-launchfaceenrollment", "التعرف على الوجه (Windows Hello)", "Facial recognition (Windows Hello)", ACCOUNTS, &["face", "facial recognition", "windows hello face", "بصمة الوجه", "التعرف على الوجه"], 0),
    p("ms-settings:signinoptions-launchfingerprintenrollment", "التعرف على بصمة الإصبع (Windows Hello)", "Fingerprint recognition (Windows Hello)", ACCOUNTS, &["fingerprint", "finger print", "بصمة الإصبع", "البصمة", "بصمة"], 0),
    p("ms-settings:signinoptions-dynamiclock", "القفل الديناميكي", "Dynamic lock", ACCOUNTS, &["dynamic lock", "auto lock", "قفل تلقائي", "القفل الديناميكي"], 0),
    p("ms-settings:workplace", "الوصول إلى العمل أو المدرسة", "Access work or school", ACCOUNTS, &["work or school", "work account", "school account", "azure ad", "حساب العمل", "حساب المدرسة", "حساب الجامعة"], 0),
    p("ms-settings:otherusers", "المستخدمون الآخرون", "Other users", ACCOUNTS, &["other users", "add user", "new user", "family & other users", "مستخدم جديد", "إضافة مستخدم", "المستخدمين", "حساب جديد"], 0),
    p("ms-settings:family-group", "العائلة", "Family", ACCOUNTS, &["family", "parental controls", "child account", "الرقابة الأبوية", "حساب طفل", "العائلة"], 22621),
    p("ms-settings:sync", "النسخ الاحتياطي لـ Windows", "Windows backup", ACCOUNTS, &["sync", "sync settings", "backup", "onedrive", "remember my apps", "المزامنة", "مزامنة الإعدادات", "نسخ احتياطي"], 0),
    p("ms-settings:backup", "النسخ الاحتياطي", "Backup", SYSTEM, &["backup", "back up", "file history", "restore files", "النسخ الاحتياطي", "نسخة احتياطية", "باك اب", "سجل الملفات"], 0),
    p("ms-settings:assignedaccess", "إعداد كشك", "Set up a kiosk", ACCOUNTS, &["kiosk", "assigned access", "كشك", "الوصول المعين"], 0),
    // Time & language.
    p("ms-settings:dateandtime", "التاريخ والوقت", "Date & time", TIME, &["date", "time", "clock", "time zone", "timezone", "sync time", "التاريخ", "الوقت", "الساعة", "المنطقة الزمنية", "التوقيت", "ضبط الساعة", "ضبط الوقت", "تغيير التاريخ"], 0),
    p("ms-settings:regionformatting", "المنطقة", "Region", TIME, &["region", "country", "regional format", "number format", "hijri", "calendar", "المنطقة", "البلد", "الدولة", "تنسيق التاريخ", "التقويم الهجري", "الأرقام العربية", "التنسيق الإقليمي"], 0),
    p("ms-settings:regionlanguage", "اللغة والمنطقة", "Language & region", TIME, &["language", "display language", "keyboard", "input language", "keyboard layout", "add language", "arabic", "english", "اللغة", "لغة العرض", "الكيبورد", "لوحة المفاتيح", "تغيير اللغة", "اللغة العربية", "إضافة لغة", "تخطيط لوحة المفاتيح", "لغة الكيبورد", "عربي", "انجليزي"], 0),
    p("ms-settings:speech", "الكلام", "Speech", TIME, &["speech", "voice", "text to speech", "narrator voice", "الكلام", "الصوت", "تحويل النص إلى كلام", "التعرف على الكلام"], 0),
    // Gaming.
    p("ms-settings:gaming-gamebar", "Xbox Game Bar", "Xbox Game Bar", GAMING, &["game bar", "xbox", "win g", "شريط الألعاب", "اكس بوكس"], 0),
    p("ms-settings:gaming-gamedvr", "اللقطات", "Captures", GAMING, &["captures", "game dvr", "record screen", "screen recording", "تسجيل الشاشة", "تسجيل الألعاب", "لقطات الشاشة"], 0),
    p("ms-settings:gaming-gamemode", "وضع اللعبة", "Game Mode", GAMING, &["game mode", "gaming", "fps", "وضع الألعاب", "الألعاب", "وضع اللعب"], 15007),
    p("ms-settings:gaming-xboxnetworking", "شبكات Xbox", "Xbox Networking", GAMING, &["xbox networking", "nat type", "شبكة اكس بوكس"], 0).until(WIN10_LAST),
    // Accessibility.
    p("ms-settings:easeofaccess-display", "حجم النص", "Text size", ACCESS, &["text size", "make text bigger", "font size", "bigger text", "حجم الخط", "تكبير الخط", "تكبير النص", "حجم النص", "خط كبير"], 0),
    p("ms-settings:easeofaccess-visualeffects", "التأثيرات المرئية", "Visual effects", ACCESS, &["visual effects", "animation effects", "transparency effects", "scrollbars", "التأثيرات المرئية", "الحركات", "الرسوم المتحركة"], WIN11),
    p("ms-settings:easeofaccess-otheroptions", "خيارات أخرى", "Other options", ACCESS, &["animations", "visual options", "الرسوم المتحركة", "خيارات مرئية"], 0).until(WIN10_LAST),
    p("ms-settings:easeofaccess-mousepointer", "مؤشر الماوس واللمس", "Mouse pointer and touch", ACCESS, &["mouse pointer", "cursor size", "pointer color", "pointer size", "مؤشر الماوس", "حجم المؤشر", "لون المؤشر", "سهم الماوس"], 0),
    p("ms-settings:easeofaccess-cursor", "مؤشر النص", "Text cursor", ACCESS, &["text cursor", "caret", "cursor thickness", "مؤشر الكتابة", "مؤشر النص"], 17040),
    p("ms-settings:easeofaccess-magnifier", "المكبر", "Magnifier", ACCESS, &["magnifier", "zoom", "magnify", "المكبر", "مكبر الشاشة", "تكبير الشاشة", "عدسة"], 0),
    p("ms-settings:easeofaccess-colorfilter", "عوامل تصفية الألوان", "Color filters", ACCESS, &["color filters", "colour blind", "grayscale", "invert colors", "عمى الألوان", "تدرج الرمادي", "عكس الألوان", "فلتر الألوان", "أبيض وأسود"], 16215),
    p("ms-settings:easeofaccess-highcontrast", "نسق التباين", "Contrast themes", ACCESS, &["high contrast", "contrast themes", "contrast", "التباين العالي", "تباين"], 0),
    p("ms-settings:easeofaccess-narrator", "الراوي", "Narrator", ACCESS, &["narrator", "screen reader", "read aloud", "الراوي", "قارئ الشاشة", "القراءة بصوت عالٍ"], 0),
    p("ms-settings:easeofaccess-audio", "الصوت (إمكانية الوصول)", "Audio (accessibility)", ACCESS, &["mono audio", "flash screen for notifications", "صوت أحادي", "وميض الشاشة"], 17035),
    p("ms-settings:easeofaccess-closedcaptioning", "التسميات التوضيحية", "Captions", ACCESS, &["captions", "subtitles", "live captions", "closed captions", "الترجمة", "الترجمة المصاحبة", "التسميات التوضيحية", "ترجمة الفيديو"], 0),
    p("ms-settings:easeofaccess-speechrecognition", "الكلام (إمكانية الوصول)", "Speech (accessibility)", ACCESS, &["voice access", "voice typing", "speech recognition", "dictation", "الكتابة بالصوت", "الإملاء الصوتي", "التحكم بالصوت", "التعرف على الصوت"], 17035),
    p("ms-settings:easeofaccess-keyboard", "لوحة المفاتيح (إمكانية الوصول)", "Keyboard (accessibility)", ACCESS, &["sticky keys", "filter keys", "toggle keys", "on-screen keyboard", "osk", "print screen key", "المفاتيح الثابتة", "مفاتيح التصفية", "لوحة المفاتيح على الشاشة", "كيبورد على الشاشة", "زر الطباعة"], 0),
    p("ms-settings:easeofaccess-mouse", "الماوس (إمكانية الوصول)", "Mouse keys", ACCESS, &["mouse keys", "numeric keypad mouse", "مفاتيح الماوس"], 0),
    p("ms-settings:easeofaccess-eyecontrol", "التحكم بالعين", "Eye control", ACCESS, &["eye control", "eye tracking", "تتبع العين", "التحكم بالعين"], 17035),
    // Privacy & security.
    p("ms-settings:windowsdefender", "أمن Windows", "Windows Security", PRIVACY, &["windows security", "defender", "antivirus", "virus", "firewall", "protection", "malware", "أمان ويندوز", "الحماية", "مكافح الفيروسات", "فيروسات", "الجدار الناري", "ديفندر", "الأمان"], 0),
    p("ms-settings:findmydevice", "العثور على جهازي", "Find my device", PRIVACY, &["find my device", "lost laptop", "track device", "العثور على جهازي", "تتبع الجهاز", "فقدان الجهاز"], 0),
    p("ms-settings:privacy", "الخصوصية العامة", "General privacy", PRIVACY, &["privacy", "advertising id", "ads", "الخصوصية", "معرف الإعلان", "الإعلانات"], 0),
    p("ms-settings:privacy-location", "الموقع", "Location", PRIVACY, &["location", "gps", "location services", "الموقع", "تحديد الموقع", "خدمات الموقع", "الموقع الجغرافي", "جي بي اس"], 0),
    p("ms-settings:privacy-webcam", "الكاميرا (الخصوصية)", "Camera privacy", PRIVACY, &["camera", "webcam", "camera access", "camera permission", "الكاميرا", "الوصول إلى الكاميرا", "إذن الكاميرا", "كاميرا الويب"], 0),
    p("ms-settings:privacy-microphone", "الميكروفون (الخصوصية)", "Microphone privacy", PRIVACY, &["microphone", "mic", "microphone access", "microphone permission", "الميكروفون", "المايك", "المايكروفون", "إذن الميكروفون"], 0),
    p("ms-settings:privacy-notifications", "الإشعارات (الخصوصية)", "Notifications privacy", PRIVACY, &["notification access", "الوصول إلى الإشعارات"], 0),
    p("ms-settings:privacy-speech", "الكلام (الخصوصية)", "Speech privacy", PRIVACY, &["online speech recognition", "speech privacy", "التعرف على الكلام عبر الإنترنت"], 0),
    p("ms-settings:privacy-speechtyping", "تخصيص الكتابة اليدوية والكتابة", "Inking & typing personalization", PRIVACY, &["inking", "typing personalization", "custom dictionary", "القاموس الشخصي", "تخصيص الكتابة"], 0),
    p("ms-settings:privacy-voiceactivation", "التنشيط الصوتي", "Voice activation", PRIVACY, &["voice activation", "hey cortana", "التنشيط بالصوت", "التنشيط الصوتي"], 17063),
    p("ms-settings:privacy-accountinfo", "معلومات الحساب (الخصوصية)", "Account info privacy", PRIVACY, &["account info access", "الوصول لمعلومات الحساب"], 0),
    p("ms-settings:privacy-contacts", "جهات الاتصال", "Contacts", PRIVACY, &["contacts", "contacts access", "جهات الاتصال", "الأسماء"], 0),
    p("ms-settings:privacy-calendar", "التقويم (الخصوصية)", "Calendar privacy", PRIVACY, &["calendar access", "الوصول إلى التقويم"], 0),
    p("ms-settings:privacy-phonecalls", "المكالمات الهاتفية", "Phone calls", PRIVACY, &["phone calls", "calls", "المكالمات", "مكالمات"], 0),
    p("ms-settings:privacy-callhistory", "سجل المكالمات", "Call history", PRIVACY, &["call history", "call log", "سجل المكالمات"], 0),
    p("ms-settings:privacy-email", "البريد الإلكتروني (الخصوصية)", "Email privacy", PRIVACY, &["email access", "الوصول إلى البريد"], 0),
    p("ms-settings:privacy-tasks", "المهام (الخصوصية)", "Tasks privacy", PRIVACY, &["tasks access", "الوصول إلى المهام"], 0),
    p("ms-settings:privacy-messaging", "المراسلة", "Messaging", PRIVACY, &["messaging", "sms", "text messages", "الرسائل", "رسائل sms", "المراسلة"], 0),
    p("ms-settings:privacy-radios", "أجهزة الراديو", "Radios", PRIVACY, &["radios", "radio control", "التحكم بالراديو", "أجهزة الراديو"], 0),
    p("ms-settings:privacy-customdevices", "أجهزة أخرى", "Other devices", PRIVACY, &["other devices", "unpaired devices", "أجهزة أخرى"], 0),
    p("ms-settings:privacy-feedback", "التشخيص والملاحظات", "Diagnostics & feedback", PRIVACY, &["diagnostics", "telemetry", "feedback", "diagnostic data", "التشخيص", "الملاحظات", "بيانات التشخيص", "القياس عن بعد"], 0),
    p("ms-settings:privacy-activityhistory", "محفوظات النشاط", "Activity history", PRIVACY, &["activity history", "timeline", "سجل النشاط", "محفوظات النشاط", "المخطط الزمني"], 17040),
    p("ms-settings:privacy-backgroundapps", "تطبيقات الخلفية", "Background apps", PRIVACY, &["background apps", "apps running in background", "battery drain", "تطبيقات الخلفية", "التطبيقات في الخلفية", "تشغيل في الخلفية"], 0).until(WIN10_LAST),
    p("ms-settings:privacy-appdiagnostics", "تشخيص التطبيقات", "App diagnostics", PRIVACY, &["app diagnostics", "تشخيص التطبيقات"], 0),
    p("ms-settings:privacy-automaticfiledownloads", "التنزيلات التلقائية للملفات", "Automatic file downloads", PRIVACY, &["automatic file downloads", "التنزيل التلقائي"], 0),
    p("ms-settings:privacy-documents", "المستندات (الخصوصية)", "Documents privacy", PRIVACY, &["documents access", "الوصول إلى المستندات", "المستندات"], 17063),
    p("ms-settings:privacy-pictures", "الصور (الخصوصية)", "Pictures privacy", PRIVACY, &["pictures access", "الوصول إلى الصور", "الصور"], 17063),
    p("ms-settings:privacy-videos", "مقاطع الفيديو (الخصوصية)", "Videos privacy", PRIVACY, &["videos access", "الوصول إلى الفيديو", "الفيديوهات"], 17063),
    p("ms-settings:privacy-broadfilesystemaccess", "نظام الملفات", "File system", PRIVACY, &["file system access", "الوصول إلى نظام الملفات"], 17063),
    // Search.
    p("ms-settings:search-permissions", "أذونات البحث", "Search permissions", SEARCH, &["safe search", "search history", "cloud search", "البحث الآمن", "سجل البحث", "أذونات البحث"], 18362),
    p("ms-settings:cortana-windowssearch", "البحث في Windows", "Searching Windows", SEARCH, &["search indexing", "indexer", "enhanced search", "find my files", "فهرسة البحث", "الفهرسة", "البحث عن الملفات", "البحث في ويندوز"], 18362),
    // Windows Update.
    p("ms-settings:windowsupdate", "تحديث Windows", "Windows Update", UPDATE, &["windows update", "update", "updates", "check for updates", "upgrade", "تحديث", "التحديثات", "تحديث ويندوز", "التحقق من وجود تحديثات", "ابديت", "تحديثات"], 0),
    p("ms-settings:windowsupdate-history", "محفوظات التحديث", "Update history", UPDATE, &["update history", "uninstall updates", "installed updates", "سجل التحديثات", "إلغاء تثبيت التحديثات", "التحديثات المثبتة"], 0),
    p("ms-settings:windowsupdate-options", "الخيارات المتقدمة للتحديث", "Advanced update options", UPDATE, &["advanced options", "pause updates", "metered updates", "إيقاف التحديثات", "تأجيل التحديثات", "إيقاف التحديث مؤقتا", "خيارات متقدمة"], 0),
    p("ms-settings:windowsupdate-activehours", "ساعات النشاط", "Active hours", UPDATE, &["active hours", "restart time", "ساعات النشاط", "ساعات الاستخدام"], 0),
    p("ms-settings:windowsupdate-restartoptions", "جدولة إعادة التشغيل", "Restart options", UPDATE, &["restart options", "schedule restart", "جدولة إعادة التشغيل"], 0),
    p("ms-settings:windowsupdate-optionalupdates", "التحديثات الاختيارية", "Optional updates", UPDATE, &["optional updates", "driver updates", "drivers", "تحديثات اختيارية", "تحديث التعريفات", "التعريفات", "درايفرات"], 19041),
    p("ms-settings:delivery-optimization", "تحسين التسليم", "Delivery Optimization", UPDATE, &["delivery optimization", "download bandwidth", "upload limit", "تحسين التوصيل", "سرعة تحميل التحديثات"], 0),
    p("ms-settings:windowsinsider", "برنامج Windows Insider", "Windows Insider Program", UPDATE, &["insider", "preview builds", "beta", "dev channel", "النسخ التجريبية", "إنسايدر", "نسخة تجريبية"], 0),
    // Classic tools, opened by the shell by name.
    p("control", "لوحة التحكم", "Control Panel", TOOLS, &["control panel", "control", "كنترول بانل", "لوحة التحكم", "لوحه التحكم"], 0),
    p("devmgmt.msc", "إدارة الأجهزة", "Device Manager", TOOLS, &["device manager", "drivers", "hardware", "devmgmt", "مدير الأجهزة", "إدارة الأجهزة", "التعريفات", "تعريف", "الهاردوير"], 0),
    p("diskmgmt.msc", "إدارة الأقراص", "Disk Management", TOOLS, &["disk management", "partition", "format drive", "diskmgmt", "إدارة الأقراص", "تقسيم الهارد", "البارتشن", "الأقسام", "تهيئة القرص"], 0),
    p("services.msc", "الخدمات", "Services", TOOLS, &["services", "service", "الخدمات", "خدمات ويندوز"], 0),
    p("taskmgr", "إدارة المهام", "Task Manager", TOOLS, &["task manager", "taskmgr", "processes", "end task", "performance", "مدير المهام", "إدارة المهام", "ادارة المهام", "العمليات", "إنهاء مهمة", "تاسك مانجر"], 0),
    p("cleanmgr", "تنظيف القرص", "Disk Cleanup", TOOLS, &["disk cleanup", "cleanmgr", "clean disk", "temp files", "تنظيف القرص", "تنظيف الهارد", "الملفات المؤقتة", "تنظيف"], 0),
    p("msconfig", "تكوين النظام", "System Configuration", TOOLS, &["system configuration", "msconfig", "boot options", "safe boot", "تكوين النظام", "إعداد النظام", "خيارات الإقلاع", "الإقلاع الآمن"], 0),
    p("regedit", "محرر التسجيل", "Registry Editor", TOOLS, &["registry", "registry editor", "regedit", "الريجستري", "السجل", "محرر السجل"], 0),
    p("ncpa.cpl", "اتصالات الشبكة", "Network Connections", TOOLS, &["network connections", "network adapters", "adapter settings", "ncpa", "dns", "ip address", "اتصالات الشبكة", "محولات الشبكة", "كرت الشبكة", "تغيير dns", "عنوان ip"], 0),
    p("appwiz.cpl", "البرامج والميزات", "Programs and Features", TOOLS, &["programs and features", "uninstall a program", "add remove programs", "appwiz", "البرامج والميزات", "إلغاء تثبيت برنامج", "حذف برنامج", "إضافة أو إزالة البرامج"], 0),
    p("sysdm.cpl", "خصائص النظام", "System Properties", TOOLS, &["system properties", "environment variables", "virtual memory", "computer name", "system restore", "sysdm", "خصائص النظام", "متغيرات البيئة", "الذاكرة الظاهرية", "استعادة النظام", "نقطة استعادة"], 0),
    p("firewall.cpl", "جدار حماية Windows Defender", "Windows Defender Firewall", TOOLS, &["firewall", "windows firewall", "الجدار الناري", "جدار الحماية", "فايروول"], 0),
    p("powercfg.cpl", "خيارات الطاقة", "Power Options", TOOLS, &["power options", "power plan", "high performance", "powercfg", "خيارات الطاقة", "خطة الطاقة", "الأداء العالي"], 0),
    p("mmsys.cpl", "الصوت (لوحة التحكم)", "Sound (Control Panel)", TOOLS, &["sound control panel", "playback devices", "recording devices", "mmsys", "أجهزة التشغيل", "أجهزة التسجيل", "إعدادات الصوت الكلاسيكية"], 0),
    p("main.cpl", "خصائص الماوس", "Mouse Properties", TOOLS, &["mouse properties", "double click speed", "pointer", "خصائص الماوس", "سرعة النقر المزدوج", "مؤشرات الماوس"], 0),
    p("eventvwr.msc", "عارض الأحداث", "Event Viewer", TOOLS, &["event viewer", "event log", "logs", "eventvwr", "عارض الأحداث", "سجل الأحداث", "السجلات"], 0),
    p("perfmon", "مراقب الأداء", "Performance Monitor", TOOLS, &["performance monitor", "perfmon", "مراقب الأداء", "مراقبة الأداء"], 0),
    p("resmon", "مراقب الموارد", "Resource Monitor", TOOLS, &["resource monitor", "resmon", "مراقب الموارد", "استهلاك الموارد"], 0),
    p("dxdiag", "أداة تشخيص DirectX", "DirectX Diagnostic Tool", TOOLS, &["dxdiag", "directx", "graphics card info", "دايركت اكس", "معلومات كرت الشاشة"], 0),
    p("winver", "حول Windows", "About Windows", TOOLS, &["winver", "windows version", "build number", "إصدار ويندوز", "رقم البناء", "نسخة ويندوز"], 0),
    p("compmgmt.msc", "إدارة الكمبيوتر", "Computer Management", TOOLS, &["computer management", "compmgmt", "إدارة الكمبيوتر", "إدارة الحاسوب"], 0),
    p("taskschd.msc", "برنامج جدولة المهام", "Task Scheduler", TOOLS, &["task scheduler", "scheduled tasks", "taskschd", "جدولة المهام", "المهام المجدولة"], 0),
    p("msinfo32", "معلومات النظام", "System Information", TOOLS, &["system information", "msinfo", "msinfo32", "hardware info", "معلومات النظام", "مواصفات الجهاز"], 0),
    p("inetcpl.cpl", "خيارات الإنترنت", "Internet Options", TOOLS, &["internet options", "internet properties", "inetcpl", "خيارات الانترنت", "خصائص الانترنت"], 0),
    p("timedate.cpl", "التاريخ والوقت (لوحة التحكم)", "Date and Time (Control Panel)", TOOLS, &["timedate", "internet time", "التاريخ والوقت", "توقيت الانترنت"], 0),
    p("intl.cpl", "المنطقة (لوحة التحكم)", "Region (Control Panel)", TOOLS, &["intl", "region control panel", "system locale", "unicode", "non-unicode", "لغة البرامج غير يونيكود", "الإعدادات الإقليمية", "لغة النظام", "حل مشكلة الحروف العربية"], 0),
    p("charmap", "خريطة الأحرف", "Character Map", TOOLS, &["character map", "charmap", "symbols", "special characters", "خريطة الأحرف", "الرموز", "رموز خاصة"], 0),
    p("osk", "لوحة المفاتيح على الشاشة", "On-Screen Keyboard", TOOLS, &["on-screen keyboard", "osk", "virtual keyboard", "لوحة المفاتيح على الشاشة", "كيبورد الشاشة", "كيبورد افتراضي"], 0),
    p("mstsc", "اتصال سطح المكتب البعيد", "Remote Desktop Connection", TOOLS, &["remote desktop connection", "mstsc", "rdp", "سطح المكتب البعيد", "اتصال عن بعد"], 0),
    p("rstrui", "استعادة النظام", "System Restore", TOOLS, &["system restore", "restore point", "rstrui", "استعادة النظام", "نقطة استعادة", "استرجاع النظام"], 0),
    p("dfrgui", "إلغاء تجزئة محركات الأقراص وتحسينها", "Defragment and Optimize Drives", TOOLS, &["defragment", "defrag", "optimize drives", "trim", "إلغاء التجزئة", "تحسين الأقراص", "ديفراج"], 0),
    p("mdsched", "تشخيص ذاكرة Windows", "Windows Memory Diagnostic", TOOLS, &["memory diagnostic", "ram test", "mdsched", "فحص الرام", "تشخيص الذاكرة"], 0),
    p("optionalfeatures", "ميزات Windows", "Windows Features", TOOLS, &["windows features", "turn windows features on or off", "hyper-v", "wsl", ".net framework", "ميزات ويندوز", "تشغيل ميزات ويندوز", "نت فريم ورك"], 0),
    p("control printers", "الأجهزة والطابعات", "Devices and Printers", TOOLS, &["devices and printers", "الأجهزة والطابعات"], 0),
    p("control userpasswords2", "حسابات المستخدمين (متقدم)", "User Accounts (advanced)", TOOLS, &["netplwiz", "userpasswords2", "auto login", "تسجيل الدخول التلقائي", "حسابات المستخدمين"], 0),
    p("lusrmgr.msc", "المستخدمون والمجموعات المحليون", "Local Users and Groups", TOOLS, &["local users", "groups", "lusrmgr", "المستخدمين المحليين", "المجموعات"], 0),
    p("gpedit.msc", "محرر نهج المجموعة المحلي", "Local Group Policy Editor", TOOLS, &["group policy", "gpedit", "policy", "نهج المجموعة", "محرر السياسات"], 0),
    p("wf.msc", "جدار الحماية مع الأمان المتقدم", "Firewall with Advanced Security", TOOLS, &["advanced firewall", "firewall rules", "inbound rules", "قواعد الجدار الناري"], 0),
    p("certmgr.msc", "الشهادات", "Certificates", TOOLS, &["certificates", "certmgr", "الشهادات"], 0),
    p("colorcpl", "إدارة الألوان", "Color Management", TOOLS, &["color management", "icc profile", "إدارة الألوان", "معايرة الألوان"], 0),
    p("dccw", "معايرة ألوان الشاشة", "Calibrate display color", TOOLS, &["calibrate", "display calibration", "dccw", "معايرة الشاشة", "معايرة الألوان"], 0),
    p("cttune", "ضبط نص ClearType", "ClearType Text Tuner", TOOLS, &["cleartype", "font smoothing", "وضوح الخط", "تنعيم الخطوط"], 0),
    p("SystemPropertiesPerformance", "خيارات الأداء", "Performance Options", TOOLS, &["performance options", "visual effects", "best performance", "خيارات الأداء", "تسريع الجهاز", "المؤثرات المرئية"], 0),
    p("SystemPropertiesProtection", "حماية النظام", "System Protection", TOOLS, &["system protection", "create restore point", "إنشاء نقطة استعادة", "حماية النظام"], 0),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn table_is_well_formed() {
        assert!(PAGES.len() >= 150, "{}", PAGES.len());
        let mut seen = HashSet::new();
        for p in PAGES {
            assert!(!p.title_ar.is_empty() && !p.title_en.is_empty() && !p.keywords.is_empty(), "{}", p.uri);
            assert!(seen.insert((p.uri, p.title_en)), "duplicate {} {}", p.uri, p.title_en);
            assert!(p.max_build == 0 || p.max_build >= p.min_build, "{}", p.uri);
            assert!(p.uri.trim() == p.uri && !p.uri.is_empty());
        }
    }

    #[test]
    fn titles_follow_language() {
        let wifi = PAGES.iter().find(|p| p.uri == "ms-settings:network-wifi").unwrap();
        assert_eq!(wifi.title(true), "Wi-Fi");
        let bt = PAGES.iter().find(|p| p.uri == "ms-settings:bluetooth").unwrap();
        assert_eq!(bt.title(false), "البلوتوث");
        assert_eq!(bt.section(true), "Bluetooth & devices");
    }

    #[test]
    fn builds() {
        let disks = PAGES.iter().find(|p| p.uri == "ms-settings:disksandvolumes").unwrap();
        assert!(!disks.available(19045) && disks.available(22631) && disks.available(0));
        let tablet = PAGES.iter().find(|p| p.uri == "ms-settings:tabletmode").unwrap();
        assert!(tablet.available(19045) && !tablet.available(22631));
    }
}
