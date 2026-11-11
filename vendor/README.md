# Vendored crates

## native-windows-gui 1.0.13 (patched)

A copy of [native-windows-gui](https://github.com/gabdube/native-windows-gui)
1.0.13 (MIT, by gdube), used through `[patch.crates-io]` in `Cargo.toml`.
Only `src/`, `build.rs`, `Cargo.toml` and `README.md` of the published crate
are kept. Changes against the published crate:

- `src/win32/high_dpi.rs`, `dpi()`: releases the screen DC it gets with
  `GetDC(NULL)` and returns 96 when the DC or the DPI cannot be read.
  Upstream leaked one DC on every call of `scale_factor()`,
  `physical_to_logical()` and everything built on them (window sizes and
  positions, the cursor position). The floating search box asks for the
  scale on every animation frame, so after a while the process ran out of
  GDI handles, `GetDC` failed, `dpi()` returned 0 and `physical_to_logical`
  panicked in `muldiv` dividing by it (issue #10).
- `src/resources/font.rs`, `Font::families()`: releases its screen DC too.
- `src/controls/tray_notification.rs`, new `TrayNotification::restore()`:
  adds the icon to the tray again (`NIM_ADD` with the builder's callback
  message, icon, tooltip and visibility; `NIM_MODIFY` if it is still there).
  Upstream only ever adds the icon in the builder and modifies it afterwards,
  so after Explorer restarts (`TaskbarCreated`) the icon was gone for good.
  Called from `src/ui/tray.rs`.

To update: copy the new published crate over this folder and re-apply the
changes above (search for `ReleaseDC` and `pub fn restore`).
