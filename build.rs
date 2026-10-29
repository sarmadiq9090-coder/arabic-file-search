use embed_manifest::manifest::{ActiveCodePage, DpiAwareness};
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        // Common Controls v6 (modern look for the list view), DPI awareness and UTF-8 code page.
        embed_manifest(
            new_manifest("Bahth.FastSearch")
                .dpi_awareness(DpiAwareness::System)
                .active_code_page(ActiveCodePage::Utf8),
        )
        .expect("unable to embed manifest");

        // Application icon as resource id 1 (shown by Explorer, taskbar and shortcuts).
        embed_resource::compile("assets/bahth.rc", embed_resource::NONE)
            .manifest_required()
            .expect("unable to embed icon resource");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/bahth.rc");
    println!("cargo:rerun-if-changed=assets/bahth.ico");
}
