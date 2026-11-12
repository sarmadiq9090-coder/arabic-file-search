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

        // Application icon as resource id 1 (shown by Explorer, taskbar and
        // shortcuts) and the version resource: Task Manager, the taskbar and
        // Explorer's Details tab show its FileDescription, "InstaFind",
        // instead of the file name "bahth". The exe keeps its name
        // (`bahth.exe`: every installed copy's updater looks for it).
        let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
        let rc = out.join("bahth.rc");
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        let version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
        std::fs::write(&rc, resource_script(&dir, &version)).expect("unable to write the resource script");
        embed_resource::compile(&rc, embed_resource::NONE).manifest_required().expect("unable to embed the resources");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/bahth.ico");
}

/// The resource script: the icon (id 1) and a version resource for
/// `version` ("0.10.1" gives 0,10,1,0). Plain ASCII, so rc.exe and windres
/// read it the same way.
fn resource_script(manifest_dir: &str, version: &str) -> String {
    let mut parts = version.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).map(|p| p.parse::<u16>().unwrap_or(0));
    let (major, minor, patch) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    let icon = format!("{manifest_dir}/assets/bahth.ico").replace('\\', "/");
    format!(
        r#"1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "CompanyName", "FastKiwi"
            VALUE "FileDescription", "InstaFind"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "bahth"
            VALUE "LegalCopyright", "Copyright (c) 2026 Sarmad. All rights reserved."
            VALUE "OriginalFilename", "bahth.exe"
            VALUE "ProductName", "InstaFind"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
    )
}
