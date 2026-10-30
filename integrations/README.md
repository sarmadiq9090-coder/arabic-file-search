# التكامل مع برامج التشغيل السريع والأدوات الأخرى

يمكن استعمال فهرس InstaFind — «بحث سريع» (بمطابقته العربية: الهمزات والتاء المربوطة والألف المقصورة والتشكيل) من خارج البرنامج، عبر وضع سطر الأوامر في `bahth.exe`:

```
bahth.exe -s "فاتورة ext:pdf" --json -n 20
```

يطبع مصفوفة JSON بعناصر `{path, name, size, modified, is_dir}`. رمز الخروج: 0 نتائج، 1 لا نتائج، 2 خطأ (غالباً لا يوجد فهرس بعد: افتح البرنامج مرة لبنائه).
مسار التثبيت الافتراضي: `%LOCALAPPDATA%\Programs\Bahth\bahth.exe`.

## Flow Launcher

المجلد [`flow-launcher/`](flow-launcher/) إضافة جاهزة بلغة Python (بروتوكول JSON-RPC، دون مكتبات خارجية).

**التثبيت:**

1. انسخ المجلد `flow-launcher` كاملاً إلى `%APPDATA%\FlowLauncher\Plugins\` (يمكن تسميته `Bahth`).
2. تأكد أن Flow Launcher يعرف مسار Python (الإعدادات ← General ← Python Path، أو يتركه ينزّله تلقائياً).
3. أعد تشغيل Flow Launcher.

**الاستخدام:** اكتب `ب فاتورة` أو `bs فاتورة`.

- Enter يفتح الملف، وقائمة السياق (Shift+Enter أو السهم الأيمن) فيها «فتح المجلد» مع تحديد الملف.
- إن لم يكن البرنامج في المسار الافتراضي، حدّد مسار `bahth.exe` من إعدادات الإضافة داخل Flow، أو بمتغير البيئة `BAHTH_EXE`.
- إن ظهرت رسالة «لا يوجد فهرس» فافتح «بحث سريع» مرة واحدة ليبني الفهرس.

اختبارات الإضافة (على أي نظام): `cd integrations/flow-launcher && python3 -m unittest -v test_main`

## PowerToys Run

إضافات PowerToys Run مكتبات C# (DLL) تُبنى مع PowerToys، فلا توجد إضافة خاصة بعد. البدائل:

- إضافة Shell المدمجة: اكتب `>` ثم الأمر، مثلاً `> bahth.exe -s فاتورة` (إن أضفت مجلد البرنامج إلى `PATH`).
- أو استعمل Flow Launcher، أو سكربت AutoHotkey أدناه مع اختصار لوحة مفاتيح.

## من الأدوات الأخرى

### Windows Terminal / موجّه الأوامر

```
"%LOCALAPPDATA%\Programs\Bahth\bahth.exe" -s "تقرير" -n 20 --sort date --desc
```

أضف ملف تعريف في Windows Terminal (الإعدادات ← Open JSON file) ليفتح نافذة بحث جاهزة:

```json
{
  "name": "بحث سريع",
  "commandline": "powershell -NoExit -Command \"Set-Alias b $env:LOCALAPPDATA\\Programs\\Bahth\\bahth.exe\""
}
```

ثم اكتب في النافذة: `b -s "فاتورة ext:pdf"`.

### PowerShell

```powershell
[Console]::OutputEncoding = [Text.Encoding]::UTF8   # حتى تُقرأ الأسماء العربية من الأنبوب صحيحة
$bahth = "$env:LOCALAPPDATA\Programs\Bahth\bahth.exe"

# أكبر 10 ملفات PDF فيها «عقد»
& $bahth -s "عقد ext:pdf" --json -n 10 --sort size --desc | ConvertFrom-Json |
    Format-Table name, size, modified

# اختر نتيجة من نافذة وافتحها
& $bahth -s "فاتورة" --json -n 50 | ConvertFrom-Json |
    Out-GridView -PassThru | ForEach-Object { Invoke-Item $_.path }

# عدد النتائج فقط، مع رمز الخروج
& $bahth -s "تقرير" --count; if ($LASTEXITCODE -eq 1) { "لا نتائج" }
```

### AutoHotkey v2

سكربت من 10 أسطر: Ctrl+Alt+B يفتح مربع بحث، ويعرض النتائج في قائمة، والنقر المزدوج يفتح الملف.

```autohotkey
#Requires AutoHotkey v2.0
^!b:: {
    ib := InputBox("اكتب كلمة البحث:", "بحث سريع")
    if (ib.Result != "OK" || ib.Value = "")
        return
    exe := EnvGet("LOCALAPPDATA") "\Programs\Bahth\bahth.exe", tmp := A_Temp "\bahth-results.txt"
    RunWait(A_ComSpec ' /c ""' exe '" -s "' ib.Value '" -n 30 > "' tmp '""', , "Hide")
    g := Gui(, "نتائج: " ib.Value), lb := g.AddListBox("w800 r20", StrSplit(Trim(FileRead(tmp, "UTF-8"), "`r`n"), "`n", "`r"))
    lb.OnEvent("DoubleClick", (*) => Run(lb.Text)), g.Show()
}
```

احفظه بترميز UTF-8 (مع BOM) باسم `bahth.ahk` وشغّله. الناتج يُكتب إلى ملف مؤقت ثم يُقرأ بترميز UTF-8، لأن قراءة الأنبوب مباشرة في AutoHotkey تستعمل صفحة ترميز الكونسول فتفسد الحروف العربية.

---

## English

InstaFind's Arabic-aware index (hamza forms, ta marbuta, alef maqsura, diacritics) can be used from launchers and scripts through the CLI: `bahth.exe -s "query" --json -n 20` prints a JSON array of `{path, name, size, modified, is_dir}`; exit code 0 = results, 1 = none, 2 = error (usually no index yet: open the app once). Default install path: `%LOCALAPPDATA%\Programs\Bahth\bahth.exe`.

- **Flow Launcher:** copy `flow-launcher/` into `%APPDATA%\FlowLauncher\Plugins\` and restart Flow. Keywords `ب` and `bs`. Enter opens the file; the context menu opens its folder. Set the exe path in the plugin settings or the `BAHTH_EXE` environment variable if it is not installed in the default place. Tests: `python3 -m unittest -v test_main`.
- **PowerToys Run:** plugins are C# DLLs, so there is none yet. Use the built-in Shell plugin (`> bahth.exe -s ...`), Flow Launcher, or the AutoHotkey script above.
- **PowerShell:** set `[Console]::OutputEncoding = [Text.Encoding]::UTF8`, then pipe `--json` into `ConvertFrom-Json`.
- **AutoHotkey:** redirect output to a file and read it with `FileRead(path, "UTF-8")`; reading the pipe directly uses the console code page and garbles Arabic.
