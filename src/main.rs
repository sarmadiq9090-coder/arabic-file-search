//! بحث سريع — fast file search for Windows with Arabic support and search
//! inside documents (txt, docx, pdf, xlsx, pptx, odt, rtf, doc…).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

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

// Command-line interface: parsing and output are platform independent; the
// console glue inside is Windows-only.
#[cfg(any(windows, test))]
mod cli;

// Platform-independent logic: used by the Windows UI and unit-tested on every
// platform (the network/installer parts of `update` are Windows-only inside).
#[cfg(any(windows, test))]
mod extract;
#[cfg(any(windows, test))]
mod history;
#[cfg(any(windows, test))]
mod index;
#[cfg(any(windows, test))]
mod layout;
#[cfg(any(windows, test))]
mod normalize;
#[cfg(any(windows, test))]
mod search;
#[cfg(any(windows, test))]
mod settings;
#[cfg(any(windows, test))]
mod update;

#[cfg(windows)]
fn main() {
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
