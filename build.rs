use embed_manifest::manifest::{ActiveCodePage, DpiAwareness};
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    // Common Controls v6 (modern look for the list view), DPI awareness and UTF-8 code page.
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_manifest(
            new_manifest("Bahth.FastSearch")
                .dpi_awareness(DpiAwareness::System)
                .active_code_page(ActiveCodePage::Utf8),
        )
        .expect("unable to embed manifest");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
