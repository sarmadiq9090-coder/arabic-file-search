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

        // An 8 MB main-thread stack (Windows' default is 1 MB). The UI thread
        // runs window procedures inside other window procedures (menus,
        // message boxes, the quick search inside the floating box's click),
        // and shell, IME and accessibility hooks on a real desktop add their
        // own frames: a stack overflow there ends the program at once.
        let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
        if msvc {
            println!("cargo:rustc-link-arg-bins=/STACK:8388608");
        } else {
            println!("cargo:rustc-link-arg-bins=-Wl,--stack,8388608");
        }

        // Application icon as resource id 1 (shown by Explorer, taskbar and shortcuts).
        embed_resource::compile("assets/bahth.rc", embed_resource::NONE)
            .manifest_required()
            .expect("unable to embed icon resource");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/bahth.rc");
    println!("cargo:rerun-if-changed=assets/bahth.ico");
}
