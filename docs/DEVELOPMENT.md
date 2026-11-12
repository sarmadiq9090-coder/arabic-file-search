# Development

Notes for building InstaFind (crate `bahth`) and finding your way around the code. Users only need the [README](../README.md).

## Build

On Windows, with Rust from <https://rustup.rs> (and the Visual Studio C++ Build Tools when rustup asks):

```
cargo build --release      # target\release\bahth.exe
cargo run --release
cargo test                 # the search, index and extraction tests also run on Linux
```

The published exes are built on Windows with MSVC and a static C runtime (`+crt-static` in [`.cargo/config.toml`](../.cargo/config.toml)), exactly as the `exe` and `exe-arm64` jobs do, so they run without the Visual C++ Redistributable:

```
rustup target add x86_64-pc-windows-msvc i686-pc-windows-msvc aarch64-pc-windows-msvc
cargo build --release --target x86_64-pc-windows-msvc    # bahth.exe (x64)
cargo build --release --target i686-pc-windows-msvc      # bahth-x86.exe (32-bit)
cargo build --release --target aarch64-pc-windows-msvc   # bahth-arm64.exe (ARM64)
python ci/check_imports.py bahth.exe                     # no VCRUNTIME / non-system DLL imports, DLLs from System32 only
```

From Linux, a mingw cross-build still works for a quick local check (`rustup target add x86_64-pc-windows-gnu`, `sudo apt-get install mingw-w64`, `cargo build --release --target x86_64-pc-windows-gnu`), but nothing published is built that way.

## Checks

Every push and pull request runs [`.github/workflows/build.yml`](../.github/workflows/build.yml): `cargo clippy --all-targets -D warnings` on Linux and Windows, `cargo test` (debug and release) on Linux and `cargo test --release` on Windows, then the MSVC builds (static CRT) of `bahth.exe` (x64) and `bahth-x86.exe` (32-bit) on Windows, a native build and `cargo test --release` of `bahth-arm64.exe` on a `windows-11-arm` runner (MSVC, static CRT), an import check of each exe ([`ci/check_imports.py`](../ci/check_imports.py): no Visual C++ runtime, no DLL outside Windows, and `DependentLoadFlags` 0x800 so Windows takes the imported DLLs from System32 and not from the exe's folder, ASLR/DEP/high-entropy ASLR and Control Flow Guard in the PE header; `--mingw-ok` for a local mingw build), and a smoke test of each exact exe on real Windows of its CPU (x86 under WOW64) ([`ci/smoke.ps1`](../ci/smoke.ps1)): the GUI indexes the runner's drives, the command line searches the index (Arabic, emoji, long names, filters), and any `crash.txt` or early exit fails the job. Two more jobs check the supply chain: `deny` runs [cargo-deny](https://embarkstudios.github.io/cargo-deny/) with [`deny.toml`](../deny.toml) (RustSec advisories, licences, sources, for the Windows targets), and `zizmor` audits the workflows ([`.github/zizmor.yml`](../.github/zizmor.yml)). Every `uses:` is pinned to a full commit SHA, and the jobs that build the published exes use no build cache.

The `ui` job ([`ci/ui-test.ps1`](../ci/ui-test.ps1)) then uses the x64 exe like a person on a real Windows desktop. It types with real keystrokes and clicks with the mouse. It covers the first-run question, the floating box, Arabic and English searches (no hamza, wrong keyboard layout, Arabizi, typos), search inside documents, the right-click and Options menus, the Ctrl+Alt+F popup (apps by Arabic name, answers, Settings pages), the Arabic interface, the close button and Alt+F4 hiding to the tray, repeated clicks on the floating box, the indexing progress in the status bar, and a copy run from Downloads moving itself to `%LOCALAPPDATA%\Programs\InstaFind` (probes). **Core** checks fail the job. **Probe** checks report missing features (`MISS`) without failing it. The table goes to the job summary, and a screenshot of every step goes to the `ui-test` artifact. The popup draws its own rows, so when `BAHTH_UI_TEST` is set it writes them to `quick-rows.txt` (and its indexing progress line to `quick-note.txt`) for the test to read. The `release` job waits for `ui` as well as the build, test and smoke jobs.

## Releases

1. Bump `version` in `Cargo.toml` and add the entry to [CHANGELOG.md](../CHANGELOG.md).
2. Write `releases/vX.Y.Z-notes.md`. The first line, `# title`, becomes the release title; the rest is the release body (Arabic first, then a short **English:** line).
3. Push to `main`. When the version has no tag yet and the smoke test passed, the `release` job publishes the very `bahth.exe`, `bahth-x86.exe` and `bahth-arm64.exe` the smoke tests ran, creates the tag and marks the GitHub Release as *latest*.

The in-app updater (`src/update.rs`) reads the latest release and downloads the asset for its own CPU: exactly **`bahth.exe`** on x64 (every installed copy since 0.3.0 looks for that name, so never rename it), `bahth-x86.exe` on 32-bit and `bahth-arm64.exe` on ARM64 (`ASSET_NAME`). Keep those names, keep releases out of draft/prerelease, and keep the repository URL. Editing a `releases/*-notes.md` file later re-syncs that release's title and body ([`release-notes.yml`](../.github/workflows/release-notes.yml)); assets are never touched.

The updater fails closed: it installs a release only when GitHub publishes a SHA-256 `digest` for the asset (it does for every upload), and refuses it otherwise, pointing to the manual download. A copy installed with Scoop (under `%SCOOP%`/`%USERPROFILE%\scoop` `apps\` or `shims\`) or winget (under `...\WinGet\Packages\` or `Links\`) never updates itself: the manual check says to run `scoop update instafind` / `winget upgrade Sarmad.InstaFind` (`update::package_manager_for`).

## Code map

| Path | What it does |
|---|---|
| `src/main.rs` | Entry point: CLI mode (`-s`) or the window |
| `src/ui/` | The Win32 UI (native-windows-gui, virtual ListView): `mod.rs` state and window, `build.rs` controls and events, `search_jobs.rs` background indexing and search, `results.rs` results, sorting and copying, `files.rs` rename and delete, `history_menu.rs`, `updates.rs`, `content_index.rs`, `keys.rs`, `startup.rs` start-up and log, `format.rs` and `shell.rs` helpers |
| `src/ui/lazy.rs` | Fast start-up: secondary windows are built and wired on first open |
| `src/ui/autostart.rs`, `quickswitch.rs`, `watching.rs`, `thumbs.rs`, `lang_menu.rs` | Start with Windows, Ctrl+Alt+G in Save/Open dialogs, live index switch, thumbnail view, language menu |
| `src/ui/quick.rs`, `floating.rs`, `tray.rs`, `surface.rs` | Search from anywhere (0.8): the quick search popup (Ctrl+Alt+F, optional Alt+Space), the translucent search box above the taskbar clock, the monochrome tray icon and menu, the new Options items and the one-time question; `surface.rs` draws them with real transparency (GDI drawn on black and white, then matted) and the Windows 11 look (rounded corners, dark mode, acrylic) |
| `src/quick.rs` | The platform-independent part of it, unit-tested on Linux: where the box and the popup go (taskbar edge, right-to-left taskbar, DPI), the fade, the grouping and order of results, the list layout, pixel maths and the tray glyph |
| `src/launcher/` | What the quick search finds besides files: apps, settings pages, open windows, answers, system commands, web shortcuts (the UI reaches its Windows-side parts through the thin `ui/launcher_shim.rs`) |
| `src/i18n.rs` | Arabic/English strings side by side (`tr!`, `trf!`), language from the registry or Windows |
| `src/index.rs` | Parallel drive indexing and the saved index (`%LOCALAPPDATA%\BahthSari\index.bin`) |
| `src/indexing_status.rs`, `src/ui/indexing.rs` | First day: the indexing progress («جارِ الفهرسة… 36,000 من 432,000 ملف») in the status bar's last part and under the quick popup's results, and "results may be incomplete while indexing" instead of "no results"; read from the scan's and the content warm-up's own counters |
| `src/relocate.rs`, `src/ui/relocate.rs` | First run from Downloads or a temp folder: the one-time offer to copy the exe to `%LOCALAPPDATA%\Programs\InstaFind\bahth.exe`, add a Start-menu shortcut, point start-with-Windows there and restart (`--moved`); never for winget, Scoop, Chocolatey or Program Files copies |
| `src/run_entry.rs` | Which exe the start-with-Windows entry starts: a copy in Downloads or a temp folder keeps an entry pointing to another copy that still exists (the moved one); unit-tested |
| `src/watch.rs`, `src/watch_pace.rs` | Live index updates (ReadDirectoryChangesW), with paced rescans after lost events |
| `src/search.rs`, `src/search/` | Query parsing, name and content search; `members.rs` one result per archive; `robustness.rs` fuzz-style tests |
| `src/normalize.rs` | Arabic normalisation for matching (hamza, ta marbuta, alef maqsura, diacritics, tatweel, Persian letters, presentation forms) |
| `src/layout.rs`, `src/franco.rs`, `src/fuzzy.rs`, `src/root.rs` | Wrong keyboard layout, Arabizi, typo tolerance and the fallback chain, triliteral roots |
| `src/extract.rs`, `src/extract/` | Text from documents; mail (eml, msg), epub, reversed-Arabic PDF fix, archives with zip-bomb limits, images of scanned PDF pages |
| `src/content_cache.rs` | Persistent content index (lz4) in `%LOCALAPPDATA%\BahthSari\content\` |
| `src/ocr.rs` | Text in images and scanned PDFs via Windows.Media.Ocr |
| `src/dupes.rs`, `src/ui/dupes_window.rs` | Duplicate finder (size, then file ends, then SHA-256) |
| `src/folder_sizes.rs`, `src/ui/disk_usage.rs` | Folder sizes from the index and the disk-usage window |
| `src/replace.rs`, `src/ui/replace_window.rs` | Find & replace in text files: encoding kept, backups, undo |
| `src/web.rs`, `src/ui/web_server.rs` | Read-only LAN search page for a phone, PIN-protected, off by default |
| `src/thumb_cache.rs`, `src/draw.rs`, `src/dragdrop.rs`, `src/icon.rs` | Thumbnail LRU, match underline, drag & drop and clipboard, icons |
| `src/history.rs`, `src/settings.rs` | Search history, saved searches, often-opened files, quick-search launch counts and pinned favourites (all in `history.json`); window, column and quick-search settings (`settings.txt`) |
| `src/update.rs` | Auto-update from GitHub Releases (WinHTTP, required SHA-256 check, fallback when the API is rate-limited, defers to winget/Scoop installs) |
| `src/crash_log.rs` | Panic hook writing `log.txt` and `crash.txt` with a backtrace |
| `src/feedback.rs` | Prefilled GitHub issue links (Options ▸ Send feedback) and the opt-in crash report offered at the next start; nothing is sent without the user (UI in `src/ui/feedback.rs`) |
| `src/rating.rs`, `src/ui/rating.rs` | Microsoft Store build only (`rating::is_packaged`, via `GetCurrentPackageFullName`): after 5 files (not folders) opened from results (`file_opens` in `settings.txt`, local only), one calm row under the results (never in the window's first minute on screen, while typing, minimized or in the background) asks «قيّم InstaFind في المتجر» / «ليس الآن»; once shown, `rating_asked=1` and it never asks again. Options and the tray menu keep "Rate in the Store" (`ms-windows-store://review/?ProductId=9MTK4KMFQ657`). **TODO (new WebView2 UI):** expose the same `rating::rating_due` / `show_now` decision and the answer (`rating_asked`) there instead of re-implementing it |
| `src/webui/`, `src/ui/webui.rs` | The new interface (1.0, phase 2, MSVC builds only, opt-in: Options ▸ «جرّب الشكل الجديد» / "Try the new look" in the classic window, shown only when WebView2 is installed, saves `new_ui=1` in `settings.txt` and swaps the windows without a restart; Settings ▸ "Back to the classic look" on the page (`classic_look`) undoes it; `--webui` or `BAHTH_WEBUI=1` force it on for one launch): the main window as a page in WebView2 (wry) inside a Win32 window, its files embedded in the exe and served by the `instafind://` protocol with a strict CSP (`assets.rs`, `assets/`); typed messages with a per-launch token and an id-based API (the page names rows, never paths; folders come from Windows' folder dialog), and the searches behind them, unit-tested on Linux (`bridge.rs`): file names with the type, size, date and folder filters and sorting, search inside documents with snippets and every matching line, rename and Recycle Bin checks, recent and saved searches, settings; the WebView2 host (`host.rs`). The classic window stays hidden behind it and is the fallback when WebView2 is missing or fails. `node src/webui/preview.cjs <dir>` (with Playwright) renders every screen in Arabic and English with synthetic files and checks CSP, requests, script errors, missing strings, menus on screen, the request fields the program accepts, the page direction and the left-to-right paths, and that a file name shows its extension left to right at its logical end (base and extension in separate isolates, matches still marked; the classic list does the same with marks, `bidi_layout::list_name`) |
| `src/cli.rs` | `bahth.exe -s …` |
| `integrations/` | Flow Launcher plugin, PowerShell, AutoHotkey and Windows Terminal examples |
| `assets/` | Icon (`python3 assets/make_icon.py` regenerates it) and the resource script |

## Known gaps

- The new interface (`src/ui/webui.rs`): its window's size and maximized state are not saved; it opens with the classic window's saved size (or where the classic window was when switching).
- All three published exes are MSVC builds and offer "Try the new look". The `ui` job tests the classic window only; the new look is checked by the `newlook` job ([`ci/newlook-test.ps1`](../ci/newlook-test.ps1), x64 exe on `windows-latest`, `release` waits for it): with `new_ui=1` the new window opens, its page loads (no WebView2 failure or blank-page fallback in `log.txt` after the 20 s watchdog), `settings.txt` keeps `new_ui=1` without `new_ui_failed=1`, nothing crashes and the window is painted; with `new_ui=0` the classic window opens and its Options menu starts with "Try the new look". Choosing it (switching without a restart) is only a probe there, and neither the way back nor the fallback after a WebView2 crash is exercised; the x86 and arm64 exes are not run with the new look. Screenshots and diagnostics are in the `newlook-test` artifact.

## Testing on Wine

The exe runs under Wine for quick checks (`LANG=C.UTF-8`, otherwise Arabic paths fail). Wine draws list items mirrored under right-to-left layout, so check list rendering on real Windows.
