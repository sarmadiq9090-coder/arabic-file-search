//! بحث سريع — fast file search for Windows with Arabic support and search
//! inside documents (txt, docx, pdf, xlsx, pptx, odt, rtf, doc…).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod dragdrop;
#[cfg(windows)]
mod draw;
mod extract;
mod history;
#[cfg(windows)]
mod icon;
mod index;
mod layout;
mod normalize;
mod search;
mod settings;
#[cfg(windows)]
mod ui;
// Not cfg(windows): the pure update logic is unit-tested on every platform;
// the network/installer parts inside are Windows-only.
mod update;

#[cfg(windows)]
fn main() {
    ui::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("This program has a Windows user interface; build it for Windows.");
}
