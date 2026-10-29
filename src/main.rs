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
mod ui;

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
    ui::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("This program has a Windows user interface; build it for Windows.");
}
