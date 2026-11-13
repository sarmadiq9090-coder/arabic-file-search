//! بحث سريع — fast file search for Windows with Arabic support and search
//! inside documents (txt, docx, pdf, xlsx, pptx, odt, rtf, doc…).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// The interface language and its `tr!` / `trf!` macros, shared by the UI and
// the CLI; declared first so the macros are visible in every module.
#[cfg(any(windows, test))]
#[macro_use]
mod i18n;

// The UI and Win32 glue only exist on Windows.
#[cfg(windows)]
mod dragdrop;
#[cfg(windows)]
mod draw;
#[cfg(windows)]
mod icon;
#[cfg(windows)]
mod ocr;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod watch;

// Command-line interface: parsing and output are platform independent; the
// console glue inside is Windows-only.
#[cfg(any(windows, test))]
mod cli;

// Platform-independent logic: used by the Windows UI and unit-tested on every
// platform (the network/installer parts of `update` are Windows-only inside).
#[cfg(any(windows, test))]
mod bidi_layout;
#[cfg(any(windows, test))]
mod cloud;
#[cfg(any(windows, test))]
mod content_cache;
#[cfg(any(windows, test))]
mod crash_log;
#[cfg(any(windows, test))]
mod dupes;
#[cfg(any(windows, test))]
mod extract;
#[cfg(any(windows, test))]
mod file_dialog;
#[cfg(any(windows, test))]
mod feedback;
#[cfg(any(windows, test))]
mod folder_sizes;
#[cfg(any(windows, test))]
mod franco;
#[cfg(any(windows, test))]
mod fuzzy;
#[cfg(any(windows, test))]
mod handover;
#[cfg(any(windows, test))]
mod history;
#[cfg(any(windows, test))]
mod index;
#[cfg(any(windows, test))]
mod index_save;
#[cfg(any(windows, test))]
mod indexing_status;
#[cfg(any(windows, test))]
mod launcher;
#[cfg(any(windows, test))]
mod layout;
#[cfg(any(windows, test))]
mod leave;
#[cfg(any(windows, test))]
mod mem_guard;
#[cfg(any(windows, test))]
mod normalize;
#[cfg(any(windows, test))]
mod quick;
#[cfg(any(windows, test))]
mod rating;
#[cfg(any(windows, test))]
mod relocate;
#[cfg(any(windows, test))]
mod replace;
#[cfg(any(windows, test))]
mod report;
#[cfg(any(windows, test))]
mod root;
#[cfg(any(windows, test))]
mod run_entry;
#[cfg(any(windows, test))]
mod search;
#[cfg(any(windows, test))]
mod suggest;
#[cfg(any(windows, test))]
mod system_files;
#[cfg(any(windows, test))]
mod settings;
#[cfg(any(windows, test))]
mod thumb_cache;
#[cfg(any(windows, test))]
mod update;
#[cfg(any(windows, test))]
mod watch_pace;
#[cfg(any(windows, test))]
mod winrt_guard;
#[cfg(any(windows, test))]
mod web;
#[cfg(any(windows, test))]
mod webui;
#[cfg(any(windows, test))]
mod window_fit;

/// Libraries loaded by name while the program runs (by this code, the
/// standard library's backtrace, the WebView2 loader, wry, or Windows DLLs
/// loading their own helpers) come from System32 only, never from the exe's
/// folder, the current folder or `PATH`: the exe is often run from Downloads,
/// where a DLL with a system name could have been planted. A library named by
/// its full path (the WebView2 runtime, COM servers such as shell extensions,
/// input methods) still loads from that path. The DLLs the exe imports are
/// covered by `/DEPENDENTLOADFLAG` (build.rs), as they load before `main`.
#[cfg(windows)]
fn restrict_dll_search() {
    use winapi::um::libloaderapi::{LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories};
    // SAFETY: a plain call with a documented flag; it changes only where
    // later LoadLibrary calls of this process look.
    unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) };
}

#[cfg(windows)]
fn main() {
    // Before anything can load a library.
    restrict_dll_search();
    // Then this, so a panic anywhere (UI or command line) leaves a report.
    crash_log::install(crash_log::default_log_path());
    crash_log::name_global_pool();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| cli::is_cli_arg(a)) {
        cli::run(&args);
    }
    ui::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("This program has a Windows user interface; build it for Windows.");
}

// Counts allocations for the tests that measure memory (`index_save`).
#[cfg(test)]
mod test_alloc;
#[cfg(test)]
#[global_allocator]
static TEST_ALLOC: test_alloc::Counting = test_alloc::Counting;
