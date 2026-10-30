<p align="center"><img src="assets/bahth-256.png" width="112" alt="InstaFind icon"></p>

<h1 align="center">InstaFind — بحث سريع</h1>

<p align="center"><b>Find any file on Windows as you type, in English or Arabic, even inside documents.</b></p>

<p align="center">
<a href="https://github.com/sarmadiq9090-coder/arabic-file-search/releases/latest"><img src="https://img.shields.io/github/v/release/sarmadiq9090-coder/arabic-file-search?color=2f81f7" alt="Release"></a>
<a href="https://github.com/sarmadiq9090-coder/arabic-file-search/releases"><img src="https://img.shields.io/github/downloads/sarmadiq9090-coder/arabic-file-search/total" alt="Downloads"></a>
<a href="https://github.com/sarmadiq9090-coder/arabic-file-search/actions/workflows/build.yml"><img src="https://github.com/sarmadiq9090-coder/arabic-file-search/actions/workflows/build.yml/badge.svg" alt="CI"></a>
<a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-green" alt="MIT"></a>
</p>

<p align="center">
<a href="https://github.com/sarmadiq9090-coder/arabic-file-search/releases/latest/download/bahth.exe"><b>⬇ Download bahth.exe</b></a> ·
<a href="CHANGELOG.md">Changelog (Arabic)</a> ·
<a href="README.md">العربية</a>
</p>

<p align="center">
<img src="docs/screenshots/search.png" width="49%" alt="Results as you type">
<img src="docs/screenshots/inside-documents.png" width="49%" alt="Searching inside documents with a text snippet">
</p>

InstaFind is a free, open-source file search tool for Windows, written in Rust. It indexes all your drives once and shows results while you type, even with millions of files. It understands Arabic the way people actually type it: `احمد` finds `أحمد`, `التقرير` finds `تقرير.pdf`, and `hgjrvdv` typed on the wrong keyboard layout finds `التقرير`.

## Download and run

1. Download [`bahth.exe`](https://github.com/sarmadiq9090-coder/arabic-file-search/releases/latest/download/bahth.exe) from the [latest release](https://github.com/sarmadiq9090-coder/arabic-file-search/releases/latest). It is a single file; nothing to install.
2. Put it in a permanent folder, for example `%LOCALAPPDATA%\Programs\Bahth\` (the path the `integrations/` plugins expect), and run it.
3. If SmartScreen warns because the file is not code-signed: **More info → Run anyway**.

The first run indexes your drives in the background (a minute or more, depending on the number of files). After that the app opens instantly and keeps the index current by itself as files are added, deleted or renamed.

- **Automatic updates**: once a day the app checks GitHub, offers the new version with one click and restarts. You can also check from **Options…**. Version 0.3.1 and later update themselves.
- **Requirements**: Windows 8 or later, 64-bit. Text recognition in images needs Windows 10 or 11 (plus the Windows language pack for each OCR language).
- **Privacy**: everything stays on your PC. The only internet connection is the update check against GitHub; phone search is off unless you turn it on.
- **Data** lives in `%LOCALAPPDATA%\BahthSari\` (index, log, settings) and `HKCU\Software\BahthSari`. To remove the app, turn off *Start with Windows* in **Options…**, then delete `bahth.exe` and that folder.

## Features

### Search
- **Instant filename search** across all drives or one folder, with ready-made filters: documents, pictures, audio, video, archives, programs, folders.
- **Real Arabic matching**: hamza forms, ta marbuta, alef maqsura, diacritics, tatweel and the `ال` prefix are ignored; Arabic-Indic digits and Persian letters are accepted.
- **Forgiving**: when nothing matches, it retries with the other keyboard layout, then Arabizi (`3abd` → عبد), then near spellings (one or two typos). `root:` searches by the Arabic triliteral root.
- **Rich query syntax**: phrases, OR, NOT, extensions, size, date modified, regex, whole word, and word proximity. See [Query syntax](#query-syntax).
- **Often-opened files first**, match underlining, sort by any column, search history and saved searches (Ctrl+H).

### Inside files
- **Documents**: docx, xlsx, xls, pptx, pdf, odt, ods, rtf, doc, txt, csv, e-mail (eml, msg), epub and source code, with a snippet of the matching text.
- **Images and scanned PDFs** through the OCR engine built into Windows, with no download.
- **Archives**: zip, 7z, tar and tar.gz are read in memory without extracting, with zip-bomb limits.
- **Persistent content index**: each document's text is extracted once in the background, so later content searches are near-instant.
- **All matches** in a file with line numbers (F4), and reversed Arabic text in PDFs is repaired.

### Tools
- **Ctrl+Alt+F** shows the app from anywhere; **Ctrl+Alt+G** in any program's Save or Open dialog jumps the dialog to the folder of a result.
- **Duplicate finder** (size, then content hash), sorted by wasted space, moves your picks to the Recycle Bin.
- **Folder sizes** in the Size column and a disk-usage window.
- **Thumbnail view** (Ctrl+T).
- **Find & replace in text files** with preview, backups and undo.
- **Search from your phone** over the home network: off by default, PIN-protected, read-only.
- **Drag & drop** results into WhatsApp, mail or Word; Ctrl+C copies the file itself; F2 renames; Delete sends to the Recycle Bin.
- **Export** results to CSV that opens correctly in Excel.
- **Command line**, a **Flow Launcher** plugin, and PowerShell / AutoHotkey examples.

### Comfort
- **English or Arabic interface**: follows the Windows display language; switch in **Options → Language / اللغة**.
- **Starts with Windows** in the background, Esc hides it to the tray, single instance.
- Relative dates, optional Arabic digits, file-type icons.
- Internal errors are written to `log.txt` and `crash.txt` instead of closing the app.

## Query syntax

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

## Keyboard

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

## Command line and integrations

The same `bahth.exe` searches from a terminal, with the same syntax, using the saved index (open the app once to build it):

```
bahth.exe -s "invoice ext:pdf"
bahth.exe -s report -n 20 --sort date --desc --scope "D:\Work"
bahth.exe -s contract --json
```

Options: `-n`/`--max`, `--sort name|path|size|date` with `--desc`, `--scope`, `--count`, `--csv`, `--json`, `--help`. Exit code 0 = results, 1 = none, 2 = error.

A Flow Launcher plugin (keyword `bs`) and PowerShell, AutoHotkey and Windows Terminal examples are in [`integrations/`](integrations/).

## Notes

- OCR is slower than document extraction. Only the first 50 pages of a scanned PDF are read, and images over 30 MB are skipped.
- Support for legacy `.doc` files is approximate.
- Bugs and ideas are welcome in [Issues](https://github.com/sarmadiq9090-coder/arabic-file-search/issues); attaching `%LOCALAPPDATA%\BahthSari\log.txt` helps.

## Developers

Building from source, the code map and the release process are in [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md). In short: `cargo build --release` on Windows; `cargo test` also runs on Linux.

## License

[MIT](LICENSE)
