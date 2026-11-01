# Development

Notes for building InstaFind (crate `bahth`) and finding your way around the code. Users only need the [README](../README.md).

## Build

On Windows, with Rust from <https://rustup.rs> (and the Visual Studio C++ Build Tools when rustup asks):

```
cargo build --release      # target\release\bahth.exe
cargo run --release
cargo test                 # the search, index and extraction tests also run on Linux
```

Cross-building from Linux, exactly as the release job does:

```
rustup target add x86_64-pc-windows-gnu i686-pc-windows-gnu
sudo apt-get install mingw-w64
cargo build --release --target x86_64-pc-windows-gnu   # bahth.exe (x64)
cargo build --release --target i686-pc-windows-gnu     # bahth-x86.exe (32-bit)
```

## Checks

Every push and pull request runs [`.github/workflows/build.yml`](../.github/workflows/build.yml): `cargo clippy --all-targets -D warnings` on Linux and Windows, `cargo test` (debug and release) on Linux and `cargo test --release` on Windows, then the gnu cross-builds of `bahth.exe` (x64) and `bahth-x86.exe` (32-bit), a native build and `cargo test --release` of `bahth-arm64.exe` on a `windows-11-arm` runner (MSVC, static CRT), and a smoke test of each exact exe on real Windows of its CPU (x86 under WOW64) ([`ci/smoke.ps1`](../ci/smoke.ps1)): the GUI indexes the runner's drives, the command line searches the index (Arabic, emoji, long names, filters), and any `crash.txt` or early exit fails the job.

## Releases

1. Bump `version` in `Cargo.toml` and add the entry to [CHANGELOG.md](../CHANGELOG.md).
2. Write `releases/vX.Y.Z-notes.md`. The first line, `# title`, becomes the release title; the rest is the release body (Arabic first, then a short **English:** line).
3. Push to `main`. When the version has no tag yet and the smoke test passed, the `release` job publishes the very `bahth.exe`, `bahth-x86.exe` and `bahth-arm64.exe` the smoke tests ran, creates the tag and marks the GitHub Release as *latest*.

The in-app updater (`src/update.rs`) reads the latest release and downloads the asset for its own CPU: exactly **`bahth.exe`** on x64 (every installed copy since 0.3.0 looks for that name, so never rename it), `bahth-x86.exe` on 32-bit and `bahth-arm64.exe` on ARM64 (`ASSET_NAME`). Keep those names, keep releases out of draft/prerelease, and keep the repository URL. Editing a `releases/*-notes.md` file later re-syncs that release's title and body ([`release-notes.yml`](../.github/workflows/release-notes.yml)); assets are never touched.

## Code map

| Path | What it does |
|---|---|
| `src/main.rs` | Entry point: CLI mode (`-s`) or the window |
| `src/ui/` | The Win32 UI (native-windows-gui, virtual ListView): `mod.rs` state and window, `build.rs` controls and events, `search_jobs.rs` background indexing and search, `results.rs` results, sorting and copying, `files.rs` rename and delete, `history_menu.rs`, `updates.rs`, `content_index.rs`, `keys.rs`, `startup.rs` start-up and log, `format.rs` and `shell.rs` helpers |
| `src/ui/lazy.rs` | Fast start-up: secondary windows are built and wired on first open |
| `src/ui/autostart.rs`, `quickswitch.rs`, `watching.rs`, `thumbs.rs`, `lang_menu.rs` | Start with Windows, Ctrl+Alt+G in Save/Open dialogs, live index switch, thumbnail view, language menu |
| `src/ui/quick.rs`, `floating.rs`, `tray.rs`, `surface.rs` | Search from anywhere (0.8): the quick search popup (Ctrl+Alt+F, optional Alt+Space), the translucent search box above the taskbar clock, the monochrome tray icon and menu, the new Options items and the one-time question; `surface.rs` draws them with real transparency (GDI drawn on black and white, then matted) and the Windows 11 look (rounded corners, dark mode, acrylic) |
| `src/quick.rs` | The platform-independent part of it, unit-tested on Linux: where the box and the popup go (taskbar edge, right-to-left taskbar, DPI), the fade, the grouping and order of results, the list layout, pixel maths and the tray glyph |
| `src/launcher/` | What the quick search finds besides files: apps, settings pages, open windows, answers, system commands, web shortcuts (`ui/launcher_shim.rs` stands in for the Windows-side parts until they land) |
| `src/i18n.rs` | Arabic/English strings side by side (`tr!`, `trf!`), language from the registry or Windows |
| `src/index.rs` | Parallel drive indexing and the saved index (`%LOCALAPPDATA%\BahthSari\index.bin`) |
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
| `src/update.rs` | Auto-update from GitHub Releases (WinHTTP, SHA-256 check, fallback when the API is rate-limited) |
| `src/crash_log.rs` | Panic hook writing `log.txt` and `crash.txt` with a backtrace |
| `src/feedback.rs` | Prefilled GitHub issue links (Options ▸ Send feedback) and the opt-in crash report offered at the next start; nothing is sent without the user (UI in `src/ui/feedback.rs`) |
| `src/cli.rs` | `bahth.exe -s …` |
| `integrations/` | Flow Launcher plugin, PowerShell, AutoHotkey and Windows Terminal examples |
| `assets/` | Icon (`python3 assets/make_icon.py` regenerates it) and the resource script |

## Testing on Wine

The exe runs under Wine for quick checks (`LANG=C.UTF-8`, otherwise Arabic paths fail). Wine draws list items mirrored under right-to-left layout, so check list rendering on real Windows.
