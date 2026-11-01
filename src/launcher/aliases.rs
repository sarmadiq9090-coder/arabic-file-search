//! Arabic names and common spellings of popular programs, so that "كروم"
//! finds Google Chrome and "وورد" finds Word. Matched against the display
//! name Windows gives the program when the apps are scanned.

use crate::normalize::normalize;

/// One program: the names it is recognised by, and the extra names it gets.
struct Alias {
    /// Matched against the display name, normalised, as whole words
    /// ("word" matches "Microsoft Word" but not "WordPad"). A leading `=`
    /// means the whole name must be equal ("=notepad" is not "Notepad++").
    names: &'static [&'static str],
    /// Added to the program's names; Arabic spellings and abbreviations.
    aliases: &'static [&'static str],
}

const fn a(names: &'static [&'static str], aliases: &'static [&'static str]) -> Alias {
    Alias { names, aliases }
}

#[rustfmt::skip]
const TABLE: &[Alias] = &[
    // Browsers.
    a(&["google chrome", "=chrome"], &["كروم", "جوجل كروم", "قوقل كروم", "غوغل كروم", "المتصفح"]),
    a(&["microsoft edge", "=edge"], &["ايدج", "إيدج", "ايدج مايكروسوفت", "المتصفح", "إنترنت"]),
    a(&["firefox"], &["فايرفوكس", "فاير فوكس", "فيرفوكس", "موزيلا", "المتصفح"]),
    a(&["opera gx"], &["أوبرا جي اكس", "اوبرا"]),
    a(&["=opera", "opera browser", "opera stable"], &["أوبرا", "اوبرا", "المتصفح"]),
    a(&["brave"], &["بريف", "المتصفح"]),
    a(&["vivaldi"], &["فيفالدي"]),
    a(&["tor browser"], &["تور"]),
    a(&["internet explorer"], &["انترنت اكسبلورر", "إنترنت إكسبلورر"]),
    a(&["yandex"], &["ياندكس"]),
    // Microsoft Office.
    a(&["=word", "microsoft word", "word 2016", "word 2019", "word 2021", "word 2013", "word 2010"], &["وورد", "ورد", "مايكروسوفت وورد", "winword"]),
    a(&["=excel", "microsoft excel", "excel 2016", "excel 2019", "excel 2021", "excel 2013", "excel 2010"], &["اكسل", "إكسل", "أكسل", "اكسيل", "جداول"]),
    a(&["powerpoint"], &["باوربوينت", "بوربوينت", "باور بوينت", "بور بوينت", "عرض تقديمي", "ppt"]),
    a(&["outlook"], &["أوتلوك", "اوتلوك", "اوت لوك", "البريد"]),
    a(&["onenote"], &["ون نوت", "وان نوت"]),
    a(&["=access", "microsoft access"], &["أكسس", "اكسس"]),
    a(&["publisher"], &["ببلشر", "بابليشر"]),
    a(&["microsoft teams", "=teams"], &["تيمز", "تيمس", "تيم"]),
    a(&["onedrive"], &["ون درايف", "وان درايف"]),
    a(&["microsoft 365", "=office", "microsoft office"], &["أوفيس", "اوفيس", "مايكروسوفت 365"]),
    a(&["skype"], &["سكايب"]),
    a(&["copilot"], &["كوبايلوت", "كوبيلوت", "مساعد"]),
    a(&["microsoft to do", "=to do"], &["المهام", "تو دو"]),
    a(&["clipchamp"], &["كليب تشامب", "مونتاج"]),
    // Communication.
    a(&["zoom"], &["زوم", "زووم"]),
    a(&["whatsapp"], &["واتساب", "واتس", "واتس اب", "واتسب", "وتساب", "whats"]),
    a(&["telegram"], &["تيليجرام", "تلغرام", "تليجرام", "تلجرام", "تيليغرام"]),
    a(&["discord"], &["ديسكورد", "ديسكور", "دسكورد"]),
    a(&["signal"], &["سيغنال", "سيجنال"]),
    a(&["viber"], &["فايبر"]),
    a(&["messenger"], &["ماسنجر", "مسنجر"]),
    a(&["slack"], &["سلاك"]),
    a(&["=mail", "mail and calendar", "windows mail"], &["البريد", "الإيميل", "ايميل", "بريد إلكتروني"]),
    a(&["thunderbird"], &["ثندربيرد", "البريد"]),
    a(&["google meet"], &["جوجل ميت", "ميت"]),
    a(&["webex"], &["ويبكس"]),
    a(&["botim"], &["بوتيم"]),
    a(&["imo"], &["ايمو", "إيمو"]),
    a(&["instagram"], &["انستقرام", "انستجرام", "إنستغرام", "انستا"]),
    a(&["facebook"], &["فيسبوك", "فيس بوك", "فيس"]),
    a(&["tiktok"], &["تيك توك", "تيكتوك"]),
    a(&["=x", "twitter"], &["تويتر", "إكس"]),
    a(&["snapchat"], &["سناب شات", "سناب"]),
    // Media.
    a(&["vlc"], &["في ال سي", "فى ال سى", "مشغل الفيديو", "vlc player"]),
    a(&["spotify"], &["سبوتيفاي", "سبوتيفي", "سبوتفاي"]),
    a(&["media player", "windows media player"], &["مشغل الوسائط", "ميديا بلاير", "مشغل الفيديو", "مشغل الصوت"]),
    a(&["movies & tv", "films & tv", "=movies and tv"], &["الأفلام والتلفزيون", "أفلام", "فيديو"]),
    a(&["groove music"], &["موسيقى", "جروف"]),
    a(&["kmplayer", "km player"], &["كي ام بلاير", "كيه ام بلاير"]),
    a(&["potplayer"], &["بوت بلاير", "بوتبلاير"]),
    a(&["gom player"], &["جوم بلاير"]),
    a(&["itunes"], &["آيتونز", "ايتونز"]),
    a(&["apple music"], &["أبل ميوزك", "ابل ميوزك"]),
    a(&["audacity"], &["أوداسيتي", "اوداسيتي"]),
    a(&["netflix"], &["نتفليكس", "نيتفليكس"]),
    a(&["youtube"], &["يوتيوب", "يوتوب"]),
    a(&["anghami"], &["أنغامي", "انغامي", "أنغامى"]),
    a(&["shahid"], &["شاهد"]),
    a(&["prime video"], &["برايم فيديو", "امازون برايم"]),
    a(&["obs studio", "=obs"], &["أو بي اس", "او بي اس", "تسجيل الشاشة", "بث"]),
    a(&["capcut"], &["كاب كات", "كابكات", "مونتاج"]),
    a(&["premiere pro"], &["بريمير", "بريميير", "مونتاج"]),
    a(&["after effects"], &["افتر افكت", "أفتر إفكتس"]),
    a(&["davinci resolve"], &["دافنشي", "دافينشي"]),
    a(&["filmora"], &["فيلمورا", "مونتاج"]),
    a(&["camtasia"], &["كامتازيا", "كامتاسيا"]),
    a(&["handbrake"], &["هاند بريك"]),
    a(&["format factory"], &["فورمات فاكتوري", "مصنع الصيغ"]),
    a(&["k-lite", "codec"], &["كودك"]),
    // Graphics and design.
    a(&["photoshop"], &["فوتوشوب", "فوتو شوب", "فتوشوب"]),
    a(&["illustrator"], &["اليستريتور", "إليستريتور", "الستريتور"]),
    a(&["lightroom"], &["لايت روم", "لايتروم"]),
    a(&["indesign"], &["إن ديزاين", "ان ديزاين"]),
    a(&["acrobat", "adobe reader", "acrobat reader"], &["أكروبات", "اكروبات", "قارئ بي دي اف", "قارئ pdf", "بي دي اف", "pdf"]),
    a(&["foxit"], &["فوكسيت", "قارئ بي دي اف", "pdf"]),
    a(&["sumatrapdf", "sumatra pdf"], &["سوماترا", "قارئ بي دي اف", "pdf"]),
    a(&["canva"], &["كانفا", "كانفا تصميم", "تصميم"]),
    a(&["figma"], &["فيجما", "فيغما"]),
    a(&["gimp"], &["جيمب", "غيمب"]),
    a(&["inkscape"], &["انكسكيب"]),
    a(&["coreldraw", "corel draw"], &["كوريل", "كورل درو", "كوريل درو"]),
    a(&["autocad"], &["أوتوكاد", "اوتوكاد", "اتوكاد"]),
    a(&["sketchup"], &["سكتش اب", "سكيتش أب"]),
    a(&["3ds max"], &["ثري دي ماكس", "ماكس"]),
    a(&["blender"], &["بلندر"]),
    a(&["=paint", "microsoft paint", "=الرسام"], &["الرسام", "رسام", "بينت", "الرسم", "mspaint"]),
    a(&["paint 3d"], &["الرسام ثلاثي الأبعاد", "بينت ثري دي"]),
    a(&["paint.net"], &["بينت دوت نت"]),
    a(&["=photos", "microsoft photos", "=الصور"], &["الصور", "صور", "عارض الصور"]),
    a(&["=camera", "windows camera", "=الكاميرا"], &["الكاميرا", "كاميرا", "كامير"]),
    a(&["snipping tool", "snip & sketch", "=أداة القطع", "=اداة القص"], &["أداة القص", "قص الشاشة", "لقطة شاشة", "سكرين شوت", "القصاصة", "أداة القطع", "snip"]),
    a(&["lightshot"], &["لايت شوت", "لقطة شاشة"]),
    a(&["sharex"], &["شير اكس", "لقطة شاشة"]),
    // Built into Windows.
    a(&["=calculator", "windows calculator", "=الحاسبة"], &["الحاسبة", "آلة حاسبة", "حاسبة", "الحاسبه", "calc", "calculator"]),
    a(&["=notepad", "windows notepad", "=المفكرة"], &["المفكرة", "مفكرة", "نوت باد", "نوتباد", "notepad"]),
    a(&["notepad++"], &["نوت باد بلس", "نوتباد بلس", "نوت باد ++"]),
    a(&["wordpad"], &["ورد باد", "وورد باد"]),
    a(&["file explorer", "=explorer", "=مستكشف الملفات"], &["مستكشف الملفات", "الملفات", "مستكشف", "المجلدات", "جهاز الكمبيوتر", "explorer"]),
    a(&["=settings", "=الإعدادات"], &["الإعدادات", "اعدادات", "settings"]),
    a(&["task manager", "=إدارة المهام"], &["مدير المهام", "إدارة المهام", "ادارة المهام", "تاسك مانجر", "taskmgr"]),
    a(&["=control panel", "=لوحة التحكم"], &["لوحة التحكم", "كنترول بانل", "control"]),
    a(&["command prompt", "=موجه الأوامر"], &["موجه الأوامر", "سي ام دي", "سطر الأوامر", "cmd"]),
    a(&["windows powershell", "=powershell", "powershell 7"], &["باورشيل", "باور شل", "بورشيل", "سطر الأوامر"]),
    a(&["windows terminal", "=terminal", "=الوحدة الطرفية"], &["الطرفية", "تيرمينال", "ترمنال", "سطر الأوامر"]),
    a(&["=microsoft store", "=store", "=المتجر"], &["المتجر", "متجر مايكروسوفت", "ستور"]),
    a(&["=clock", "alarms & clock", "=alarms and clock", "=الساعة"], &["الساعة", "المنبه", "منبه", "ساعة", "مؤقت", "ساعة إيقاف"]),
    a(&["=calendar", "=التقويم"], &["التقويم", "تقويم", "الرزنامة"]),
    a(&["=maps", "windows maps", "=الخرائط"], &["الخرائط", "خرائط"]),
    a(&["=weather", "msn weather", "=الطقس"], &["الطقس", "طقس", "حالة الطقس"]),
    a(&["=news", "microsoft news"], &["الأخبار", "اخبار"]),
    a(&["xbox"], &["اكس بوكس", "إكس بوكس", "ألعاب"]),
    a(&["sound recorder", "voice recorder", "=مسجل الصوت"], &["مسجل الصوت", "تسجيل صوت", "مسجل"]),
    a(&["sticky notes"], &["الملاحظات اللاصقة", "ملاحظات", "ستيكي نوت"]),
    a(&["remote desktop connection", "=remote desktop"], &["سطح المكتب البعيد", "اتصال سطح المكتب البعيد", "mstsc"]),
    a(&["character map"], &["خريطة الأحرف", "رموز"]),
    a(&["on-screen keyboard"], &["لوحة المفاتيح على الشاشة", "كيبورد الشاشة", "osk"]),
    a(&["magnifier"], &["المكبر", "مكبر الشاشة"]),
    a(&["narrator"], &["الراوي", "قارئ الشاشة"]),
    a(&["registry editor"], &["محرر التسجيل", "الريجستري", "regedit"]),
    a(&["disk cleanup"], &["تنظيف القرص", "تنظيف"]),
    a(&["system information"], &["معلومات النظام", "msinfo32"]),
    a(&["system configuration"], &["تكوين النظام", "msconfig"]),
    a(&["event viewer"], &["عارض الأحداث"]),
    a(&["resource monitor"], &["مراقب الموارد"]),
    a(&["=services", "=الخدمات"], &["الخدمات"]),
    a(&["device manager"], &["إدارة الأجهزة", "مدير الأجهزة"]),
    a(&["windows security", "=أمن windows"], &["أمان ويندوز", "الحماية", "مكافح الفيروسات", "ديفندر", "defender"]),
    a(&["get help"], &["الحصول على المساعدة", "مساعدة"]),
    a(&["=tips"], &["تلميحات", "نصائح"]),
    a(&["feedback hub"], &["مركز الملاحظات"]),
    a(&["phone link", "your phone"], &["ربط الهاتف", "هاتفك", "الجوال"]),
    a(&["quick assist"], &["المساعدة السريعة", "مساعدة عن بعد"]),
    a(&["windows media player legacy"], &["مشغل ويندوز ميديا"]),
    a(&["dev home"], &["ديف هوم"]),
    // Development.
    a(&["visual studio code", "=vs code", "=vscode", "=code"], &["في اس كود", "فيجوال ستوديو كود", "كود", "vscode", "vs code"]),
    a(&["visual studio 20", "=visual studio"], &["فيجوال ستوديو", "فيجول ستوديو"]),
    a(&["android studio"], &["أندرويد ستوديو", "اندرويد ستوديو"]),
    a(&["intellij"], &["انتلجي"]),
    a(&["pycharm"], &["باي تشارم", "بايتشارم"]),
    a(&["python"], &["بايثون", "بايثن"]),
    a(&["git bash"], &["جيت باش"]),
    a(&["github desktop"], &["جيت هب", "جيتهاب"]),
    a(&["postman"], &["بوستمان"]),
    a(&["docker desktop"], &["دوكر"]),
    a(&["xampp"], &["زامب"]),
    a(&["sublime text"], &["سبلايم"]),
    a(&["unity hub", "=unity"], &["يونتي", "يونيتي"]),
    a(&["matlab"], &["ماتلاب"]),
    a(&["spss"], &["اس بي اس اس"]),
    // Files and archives.
    a(&["winrar"], &["وينرار", "وين رار", "رار", "فك الضغط", "ضغط"]),
    a(&["7-zip", "7zip"], &["سفن زيب", "زيب", "فك الضغط", "ضغط"]),
    a(&["peazip"], &["بيزيب", "فك الضغط"]),
    a(&["winzip"], &["وين زيب", "فك الضغط"]),
    a(&["ccleaner"], &["سي كلينر", "تنظيف"]),
    a(&["everything"], &["إفري ثينج", "بحث الملفات"]),
    a(&["google drive"], &["جوجل درايف", "درايف"]),
    a(&["dropbox"], &["دروب بوكس", "دروببوكس"]),
    a(&["mega"], &["ميجا"]),
    a(&["internet download manager", "=idm"], &["انترنت داونلود مانجر", "داونلود مانجر", "التحميل", "idm"]),
    a(&["qbittorrent", "utorrent", "bittorrent"], &["تورنت"]),
    a(&["rufus"], &["روفوس", "فلاشة إقلاع"]),
    a(&["poweriso"], &["باور ايزو"]),
    // Remote and security.
    a(&["anydesk"], &["اني ديسك", "أني ديسك", "انى ديسك", "اني دسك", "تحكم عن بعد"]),
    a(&["teamviewer"], &["تيم فيور", "تيم فيوير", "تيمفيور", "تحكم عن بعد"]),
    a(&["rustdesk"], &["رست ديسك", "تحكم عن بعد"]),
    a(&["nordvpn", "protonvpn", "expressvpn", "windscribe", "psiphon"], &["في بي ان", "vpn"]),
    a(&["kaspersky"], &["كاسبر", "كاسبرسكي", "مكافح الفيروسات"]),
    a(&["avast"], &["افاست", "مكافح الفيروسات"]),
    a(&["avg"], &["إيه في جي", "مكافح الفيروسات"]),
    a(&["eset", "nod32"], &["ايسيت", "نود", "مكافح الفيروسات"]),
    a(&["malwarebytes"], &["مالوير بايتس", "مكافح الفيروسات"]),
    a(&["bitdefender"], &["بت ديفندر", "مكافح الفيروسات"]),
    a(&["mcafee"], &["مكافي", "مكافح الفيروسات"]),
    a(&["norton"], &["نورتون", "مكافح الفيروسات"]),
    // Games and stores.
    a(&["steam"], &["ستيم", "ألعاب"]),
    a(&["epic games"], &["ايبك جيمز", "إيبك", "ألعاب"]),
    a(&["ea app", "=origin"], &["إي إيه", "ألعاب"]),
    a(&["ubisoft connect"], &["يوبيسوفت", "ألعاب"]),
    a(&["battle.net"], &["باتل نت", "ألعاب"]),
    a(&["minecraft"], &["ماينكرافت", "ماين كرافت"]),
    a(&["roblox"], &["روبلوكس"]),
    a(&["fortnite"], &["فورتنايت"]),
    a(&["pubg"], &["ببجي", "بابجي"]),
    a(&["bluestacks"], &["بلوستاكس", "بلو ستاكس", "محاكي أندرويد"]),
    a(&["ldplayer"], &["إل دي بلاير", "محاكي أندرويد"]),
    a(&["solitaire"], &["سوليتير", "الورق"]),
    // Hardware tools.
    a(&["nvidia"], &["انفيديا", "إنفيديا", "كرت الشاشة"]),
    a(&["amd software", "radeon"], &["راديون", "إيه إم دي", "كرت الشاشة"]),
    a(&["intel graphics", "intel® graphics"], &["انتل", "كرت الشاشة"]),
    a(&["realtek audio"], &["ريلتك", "الصوت"]),
    a(&["cpu-z"], &["سي بي يو زد"]),
    a(&["hp smart"], &["إتش بي سمارت", "الطابعة"]),
    a(&["logitech"], &["لوجيتك"]),
    // Reading, learning, religion.
    a(&["kindle"], &["كيندل"]),
    a(&["duolingo"], &["دولينجو"]),
    a(&["quran", "القرآن", "المصحف"], &["القرآن", "قرآن", "المصحف", "مصحف"]),
    a(&["google earth"], &["جوجل إيرث", "جوجل ارث"]),
    a(&["evernote"], &["إيفرنوت", "ايفرنوت"]),
    a(&["notion"], &["نوشن"]),
    a(&["obsidian"], &["أوبسيديان"]),
    a(&["libreoffice"], &["ليبر أوفيس", "ليبر اوفيس"]),
    a(&["wps office", "=wps"], &["دبليو بي اس", "اوفيس"]),
    a(&["chatgpt"], &["شات جي بي تي", "شات"]),
    a(&["=claude"], &["كلود"]),
];

/// Whether `pat` (normalised) occurs in `name` (normalised) as whole words.
fn word_match(name: &str, pat: &str) -> bool {
    if let Some(exact) = pat.strip_prefix('=') {
        return name == exact;
    }
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    name.match_indices(pat).any(|(i, m)| {
        let before = name[..i].chars().next_back();
        let after = name[i + m.len()..].chars().next();
        !word(before) && !word(after)
    })
}

/// Extra names for the program called `name`: Arabic names and common
/// spellings from the table. Empty when the program is not listed.
pub fn for_name(name: &str) -> Vec<String> {
    let n = normalize(name.trim());
    let mut out: Vec<String> = Vec::new();
    for entry in TABLE {
        if entry.names.iter().any(|p| word_match(&n, &normalize(p))) {
            for al in entry.aliases {
                if normalize(al) != n && !out.iter().any(|o| o == al) {
                    out.push((*al).to_string());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popular_programs_get_arabic_names() {
        assert!(for_name("Google Chrome").contains(&"كروم".to_string()));
        assert!(for_name("Microsoft Edge").contains(&"إيدج".to_string()));
        assert!(for_name("Word").contains(&"وورد".to_string()));
        assert!(for_name("Excel 2016").contains(&"اكسل".to_string()));
        assert!(for_name("WhatsApp").contains(&"واتساب".to_string()));
        assert!(for_name("Calculator").contains(&"الحاسبة".to_string()));
        assert!(for_name("الحاسبة").contains(&"calc".to_string()));
        assert!(for_name("Snipping Tool").contains(&"قص الشاشة".to_string()));
        assert!(for_name("Visual Studio Code").contains(&"في اس كود".to_string()));
    }

    #[test]
    fn whole_words_only() {
        assert!(for_name("WordPad").iter().all(|a| a != "وورد"));
        assert!(for_name("Notepad++").iter().all(|a| a != "المفكرة"));
        assert!(for_name("NVIDIA Control Panel").iter().all(|a| a != "لوحة التحكم"));
        assert!(for_name("NVIDIA Control Panel").contains(&"انفيديا".to_string()));
        assert!(for_name("Some Unknown Tool 3000").is_empty());
    }

    #[test]
    fn table_is_well_formed() {
        assert!(TABLE.len() >= 150, "{}", TABLE.len());
        for e in TABLE {
            for p in e.names {
                let body = p.strip_prefix('=').unwrap_or(p);
                assert!(!body.is_empty() && body.trim() == body, "{p}");
            }
            assert!(!e.aliases.is_empty() && e.aliases.iter().all(|a| !a.trim().is_empty()));
        }
    }
}
