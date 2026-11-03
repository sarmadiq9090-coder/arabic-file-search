//! Microsoft Store (MSIX) mode.
//!
//! The same exe runs as a portable program and, unchanged, inside the MSIX
//! package that `packaging/msix/` builds for the Store. Packaged, Windows
//! gives the process a package identity (`GetCurrentPackageFullName`
//! succeeds) and three things work differently:
//!
//! - **Updates** come from the Store. The program's folder
//!   (`C:\Program Files\WindowsApps\…`) is read-only and owned by Windows, so
//!   the self-updater (`update.rs`) stays off and says so.
//! - **Starting with Windows** goes through the package's StartupTask
//!   (`windows.startupTask` in the manifest, [`STARTUP_TASK_ID`]). A `Run`
//!   value written by a packaged app lands in its private registry and
//!   Windows never reads it. The task starts the exe without arguments, so a
//!   sign-in launch is recognised by its activation kind instead of
//!   `--startup` ([`launched_by_startup_task`]).
//! - **Data** written under `%LOCALAPPDATA%\BahthSari` is redirected by
//!   Windows to `%LOCALAPPDATA%\Packages\<family>\LocalCache\Local\BahthSari`
//!   ([`virtualized_data_dir`]). The program keeps using the usual path and
//!   sees its own files there; Windows deletes them on uninstall. The CLI run
//!   through the `instafind.exe` alias is inside the package too, so it
//!   reads the same index.
//!
//! Global hotkeys, the tray icon, the floating box, the folder watchers and
//! the indexer need nothing special: the package is full trust
//! (`runFullTrust`), so the program runs as an ordinary desktop process of
//! the user.
//!
//! The decisions are plain functions, unit-tested on every platform; the
//! Windows calls are in `win` below.

/// `TaskId` of the `windows.startupTask` extension in
/// `packaging/msix/AppxManifest.xml` (a test checks they agree).
pub const STARTUP_TASK_ID: &str = "InstaFindStartup";
/// The command the package adds to `PATH` (`windows.appExecutionAlias`).
pub const EXECUTION_ALIAS: &str = "instafind.exe";
/// Whether the first run in the Store turns starting with Windows on once,
/// like the portable program does (`autostart::apply_default`). Off: Store
/// policy 10.2.8 wants the user's consent, so the Store version starts with
/// Windows only once the user turns it on (Options, or Settings ▸ Apps ▸
/// Startup ▸ InstaFind). Once the user turns it off there or in Task
/// Manager, Windows keeps it off (`DisabledByUser`) and the program never
/// turns it back on.
pub const ENABLE_STARTUP_ON_FIRST_RUN: bool = false;

/// `ERROR_SUCCESS` / `ERROR_INSUFFICIENT_BUFFER` / `APPMODEL_ERROR_NO_PACKAGE`.
const ERROR_SUCCESS: i32 = 0;
const ERROR_INSUFFICIENT_BUFFER: i32 = 122;
#[cfg(test)]
const APPMODEL_ERROR_NO_PACKAGE: i32 = 15700;

/// What `GetCurrentPackageFullName(&mut 0, null)` returning `status` means:
/// a packaged process gets "buffer too small" (or success), an unpackaged
/// one `APPMODEL_ERROR_NO_PACKAGE`. Anything else (an unknown error) is
/// treated as not packaged, so the portable program keeps working.
pub fn packaged_from_status(status: i32) -> bool {
    status == ERROR_SUCCESS || status == ERROR_INSUFFICIENT_BUFFER
}

/// Package family name from a package full name:
/// `Name_Version_Arch_ResourceId_PublisherId` → `Name_PublisherId`.
pub fn family_from_full_name(full: &str) -> Option<String> {
    let parts: Vec<&str> = full.split('_').collect();
    match parts.as_slice() {
        [name, _version, _arch, _resource, publisher_id] if !name.is_empty() && !publisher_id.is_empty() => {
            Some(format!("{name}_{publisher_id}"))
        }
        _ => None,
    }
}

/// Where Windows really keeps what the packaged program writes under
/// `%LOCALAPPDATA%\BahthSari`.
pub fn virtualized_data_dir(local_app_data: &str, family: &str) -> String {
    format!("{}\\Packages\\{family}\\LocalCache\\Local\\BahthSari", local_app_data.trim_end_matches(['\\', '/']))
}

/// `Windows.ApplicationModel.StartupTaskState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupState {
    Disabled,
    /// Turned off by the user in Settings or Task Manager: only they can
    /// turn it back on there.
    DisabledByUser,
    Enabled,
    DisabledByPolicy,
    EnabledByPolicy,
}

impl StartupState {
    pub fn from_raw(v: i32) -> Option<Self> {
        Some(match v {
            0 => Self::Disabled,
            1 => Self::DisabledByUser,
            2 => Self::Enabled,
            3 => Self::DisabledByPolicy,
            4 => Self::EnabledByPolicy,
            _ => return None,
        })
    }

    pub fn is_on(self) -> bool {
        matches!(self, Self::Enabled | Self::EnabledByPolicy)
    }
}

/// What to do with the startup task when the user wants it `on` or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupAction {
    /// Already as wanted.
    Nothing,
    RequestEnable,
    Disable,
    /// Windows refuses: the user turned it off in Settings / Task Manager
    /// (or a policy decides), so tell them where to change it.
    Refused,
}

pub fn startup_action(state: StartupState, on: bool) -> StartupAction {
    use StartupState::*;
    match (state, on) {
        (Enabled | EnabledByPolicy, true) | (Disabled | DisabledByUser | DisabledByPolicy, false) => StartupAction::Nothing,
        (Disabled, true) => StartupAction::RequestEnable,
        (Enabled, false) => StartupAction::Disable,
        (DisabledByUser | DisabledByPolicy, true) | (EnabledByPolicy, false) => StartupAction::Refused,
    }
}

/// Whether the first Store run should turn starting with Windows on: only
/// if the user never chose and Windows has it plainly off (not by the user).
pub fn enable_on_first_run(user_chose: bool, state: StartupState) -> bool {
    ENABLE_STARTUP_ON_FIRST_RUN && !user_chose && state == StartupState::Disabled
}

/// `Windows.ApplicationModel.Activation.ActivationKind.StartupTask`.
#[cfg_attr(not(windows), allow(dead_code))]
pub const ACTIVATION_KIND_STARTUP_TASK: i32 = 1020;

/// Shown instead of the update check in the Store version.
#[cfg(any(windows, test))]
pub fn update_message() -> String {
    trf!(
        "التحديثات تأتي من Microsoft Store.\nيحدّث المتجر InstaFind تلقائياً، أو افتح Microsoft Store ▸ المكتبة ▸ «الحصول على التحديثات».\n\nالإصدار الحالي: {}",
        "Updates come from Microsoft Store.\nThe Store updates InstaFind automatically, or open Microsoft Store ▸ Library ▸ “Get updates”.\n\nCurrent version: {}",
        env!("CARGO_PKG_VERSION")
    )
}

/// Why starting with Windows could not be turned on or off, and where the
/// user can change it.
#[cfg(any(windows, test))]
pub fn startup_refused_message(state: StartupState) -> String {
    match state {
        StartupState::DisabledByPolicy | StartupState::EnabledByPolicy => tr!(
            "إعداد التشغيل مع ويندوز تحدده سياسة المؤسسة على هذا الجهاز.",
            "Starting with Windows is set by your organisation's policy on this PC."
        )
        .to_string(),
        _ => tr!(
            "أوقفتَ تشغيل البرنامج مع ويندوز من الإعدادات. لتشغيله: الإعدادات ▸ التطبيقات ▸ بدء التشغيل ▸ InstaFind.",
            "You turned starting with Windows off in Settings. To turn it on: Settings ▸ Apps ▸ Startup ▸ InstaFind."
        )
        .to_string(),
    }
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use super::*;
    use std::sync::OnceLock;
    use windows::ApplicationModel::{AppInstance, StartupTask};
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    use windows::core::HSTRING;

    type GetCurrentPackageFullName = unsafe extern "system" fn(*mut u32, *mut u16) -> i32;

    /// `GetCurrentPackageFullName`, looked up at run time: it does not exist
    /// before Windows 8, where the portable program must still start.
    fn package_full_name_fn() -> Option<GetCurrentPackageFullName> {
        use winapi::um::libloaderapi::{GetModuleHandleW, GetProcAddress};
        let kernel32: Vec<u16> = "kernel32.dll\0".encode_utf16().collect();
        let module = unsafe { GetModuleHandleW(kernel32.as_ptr()) };
        if module.is_null() {
            return None;
        }
        let f = unsafe { GetProcAddress(module, c"GetCurrentPackageFullName".as_ptr()) };
        // SAFETY: the documented signature of GetCurrentPackageFullName.
        (!f.is_null()).then(|| unsafe { std::mem::transmute::<_, GetCurrentPackageFullName>(f) })
    }

    /// The package full name when running from an MSIX package (the Store
    /// version), `None` for the portable program. Asked once.
    pub fn package_full_name() -> Option<&'static str> {
        static NAME: OnceLock<Option<String>> = OnceLock::new();
        NAME.get_or_init(|| {
            let f = package_full_name_fn()?;
            let mut len = 0u32;
            let status = unsafe { f(&mut len, std::ptr::null_mut()) };
            if !packaged_from_status(status) {
                return None;
            }
            let mut buf = vec![0u16; len.max(1) as usize];
            let status = unsafe { f(&mut len, buf.as_mut_ptr()) };
            if status != ERROR_SUCCESS {
                // Packaged, but the name could not be read: still packaged.
                return Some(String::new());
            }
            buf.truncate(buf.iter().position(|&c| c == 0).unwrap_or(buf.len()));
            Some(String::from_utf16_lossy(&buf))
        })
        .as_deref()
    }

    /// Whether this is the Microsoft Store (MSIX) version.
    pub fn is_packaged() -> bool {
        package_full_name().is_some()
    }

    /// Runs a WinRT call on a fresh multithreaded thread, so the caller's
    /// apartment (the UI thread is single-threaded) does not matter and a
    /// blocking `.join()` cannot deadlock it.
    ///
    /// The multithreaded apartment is kept alive for the whole process
    /// (`CoIncrementMTAUsage`, once). Without that it is torn down each time
    /// one of these short-lived threads exits, and the next call goes
    /// through the `windows` crate's cached factory of the dead apartment:
    /// an access violation in combase.dll (seen in store.yml's install test).
    fn on_mta<T: Send + 'static>(f: impl FnOnce() -> windows::core::Result<T> + Send + 'static) -> Result<T, String> {
        static MTA: OnceLock<()> = OnceLock::new();
        MTA.get_or_init(|| {
            let mut cookie = std::ptr::null_mut();
            // SAFETY: an out-parameter; the cookie is never released, so the
            // apartment lives until the process exits.
            let _ = unsafe { winapi::um::combaseapi::CoIncrementMTAUsage(&mut cookie) };
        });
        std::thread::spawn(move || {
            let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
            f().map_err(|e| e.message().to_string())
        })
        .join()
        .unwrap_or_else(|_| Err("the startup task call panicked".into()))
    }

    fn task() -> windows::core::Result<StartupTask> {
        StartupTask::GetAsync(&HSTRING::from(STARTUP_TASK_ID))?.join()
    }

    /// The startup task's state; `None` when not packaged or Windows does not
    /// answer.
    pub fn startup_state() -> Option<StartupState> {
        if !is_packaged() {
            return None;
        }
        on_mta(|| task()?.State()).ok().and_then(|s| StartupState::from_raw(s.0))
    }

    /// Turns the startup task on or off. Returns the state afterwards, or
    /// `Err` with a message for the user when Windows refuses.
    pub fn set_startup(on: bool) -> Result<StartupState, String> {
        let state = startup_state().ok_or("the startup task is not available")?;
        let raw = match startup_action(state, on) {
            StartupAction::Nothing => return Ok(state),
            StartupAction::Refused => return Err(startup_refused_message(state)),
            StartupAction::RequestEnable => on_mta(|| task()?.RequestEnableAsync()?.join())?.0,
            StartupAction::Disable => {
                on_mta(|| task()?.Disable())?;
                return startup_state().ok_or_else(|| "the startup task is not available".into());
            }
        };
        let after = StartupState::from_raw(raw).ok_or("unknown startup task state")?;
        if after.is_on() == on { Ok(after) } else { Err(startup_refused_message(after)) }
    }

    /// Whether Windows started this run through the startup task (at
    /// sign-in), which passes no `--startup` argument. Asked once.
    pub fn launched_by_startup_task() -> bool {
        static AT_SIGN_IN: OnceLock<bool> = OnceLock::new();
        *AT_SIGN_IN.get_or_init(|| {
            is_packaged()
                && on_mta(|| AppInstance::GetActivatedEventArgs()?.Kind())
                    .is_ok_and(|k| k.0 == ACTIVATION_KIND_STARTUP_TASK)
        })
    }

    /// One line for `log.txt` at startup: package, startup task and where
    /// the data really is. `None` for the portable program.
    pub fn startup_log_line() -> Option<String> {
        let full = package_full_name()?;
        let data = family_from_full_name(full)
            .zip(std::env::var("LOCALAPPDATA").ok())
            .map(|(family, local)| virtualized_data_dir(&local, &family))
            .unwrap_or_default();
        Some(format!(
            "store: packaged as {full}; startup task {:?}; at sign-in {}; data under {data}; command {EXECUTION_ALIAS}",
            startup_state(),
            launched_by_startup_task()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = include_str!("../packaging/msix/AppxManifest.xml");

    #[test]
    fn package_status_codes() {
        assert!(packaged_from_status(ERROR_INSUFFICIENT_BUFFER));
        assert!(packaged_from_status(ERROR_SUCCESS));
        assert!(!packaged_from_status(APPMODEL_ERROR_NO_PACKAGE));
        // ERROR_INVALID_PARAMETER and other surprises: portable behaviour.
        assert!(!packaged_from_status(87));
        assert!(!packaged_from_status(-1));
    }

    #[test]
    fn family_name_and_data_folder() {
        let full = "FastKiwi.InstaFind_0.9.0.0_x64__8wekyb3d8bbwe";
        assert_eq!(family_from_full_name(full).as_deref(), Some("FastKiwi.InstaFind_8wekyb3d8bbwe"));
        assert_eq!(family_from_full_name("FastKiwi.InstaFind_0.9.0.0_neutral_~_abc").as_deref(), Some("FastKiwi.InstaFind_abc"));
        for bad in ["", "x", "a_b_c_d", "a_b_c_d_e_f", "_1_x64__p", "n_1_x64__"] {
            assert_eq!(family_from_full_name(bad), None, "{bad}");
        }
        let local = r"C:\Users\سرمد\AppData\Local";
        let want = r"C:\Users\سرمد\AppData\Local\Packages\F.I_p\LocalCache\Local\BahthSari";
        assert_eq!(virtualized_data_dir(local, "F.I_p"), want);
        assert_eq!(virtualized_data_dir(&format!("{local}\\"), "F.I_p"), want);
    }

    #[test]
    fn startup_states() {
        use StartupState::*;
        let all = [Disabled, DisabledByUser, Enabled, DisabledByPolicy, EnabledByPolicy];
        for (i, s) in all.iter().enumerate() {
            assert_eq!(StartupState::from_raw(i as i32), Some(*s));
        }
        assert_eq!(StartupState::from_raw(5), None);
        assert_eq!(StartupState::from_raw(-1), None);
        assert_eq!(all.iter().filter(|s| s.is_on()).count(), 2);
    }

    #[test]
    fn startup_actions_respect_the_user_and_policy() {
        use StartupAction::*;
        use StartupState::*;
        assert_eq!(startup_action(Disabled, true), RequestEnable);
        assert_eq!(startup_action(Enabled, false), Disable);
        assert_eq!(startup_action(Enabled, true), Nothing);
        assert_eq!(startup_action(Disabled, false), Nothing);
        assert_eq!(startup_action(DisabledByUser, false), Nothing);
        // Only the user (Settings / Task Manager) or the policy can undo these.
        assert_eq!(startup_action(DisabledByUser, true), Refused);
        assert_eq!(startup_action(DisabledByPolicy, true), Refused);
        assert_eq!(startup_action(EnabledByPolicy, false), Refused);
        assert_eq!(startup_action(EnabledByPolicy, true), Nothing);
    }

    #[test]
    fn first_run_never_overrides_a_choice() {
        use StartupState::*;
        assert_eq!(enable_on_first_run(false, Disabled), ENABLE_STARTUP_ON_FIRST_RUN);
        assert!(!enable_on_first_run(true, Disabled));
        for s in [DisabledByUser, DisabledByPolicy, Enabled, EnabledByPolicy] {
            assert!(!enable_on_first_run(false, s), "{s:?}");
        }
    }

    #[test]
    fn messages_name_the_store_and_settings() {
        // Tests run in Arabic (i18n::detect).
        let m = update_message();
        assert!(m.contains("Microsoft Store") && m.contains("التحديثات") && m.contains(env!("CARGO_PKG_VERSION")), "{m}");
        let m = startup_refused_message(StartupState::DisabledByUser);
        // Named as Settings ▸ Apps ▸ Startup lists it: the manifest's
        // StartupTask DisplayName, from identity.json's displayName.
        assert!(m.contains("الإعدادات") && m.contains("بدء التشغيل ▸ InstaFind"), "{m}");
        assert!(startup_refused_message(StartupState::DisabledByPolicy).contains("سياسة"));
    }

    /// The attribute values of every `<tag … attr="…">` in the manifest.
    fn attrs(tag: &str, attr: &str) -> Vec<String> {
        use quick_xml::events::Event;
        let mut reader = quick_xml::Reader::from_str(MANIFEST);
        let mut out = Vec::new();
        loop {
            match reader.read_event().expect("AppxManifest.xml is well-formed XML") {
                Event::Start(e) | Event::Empty(e) if e.name().as_ref() == tag => {
                    for a in e.attributes() {
                        let a = a.unwrap();
                        if a.key.as_ref() == attr {
                            out.push(a.value.to_string());
                        }
                    }
                }
                Event::Eof => return out,
                _ => {}
            }
        }
    }

    #[test]
    fn manifest_agrees_with_the_program() {
        assert_eq!(attrs("desktop:StartupTask", "TaskId"), [STARTUP_TASK_ID]);
        assert_eq!(attrs("desktop:ExecutionAlias", "Alias"), [EXECUTION_ALIAS]);
        // Every entry point is the one exe the release publishes, as full trust.
        assert_eq!(attrs("Application", "Executable"), ["bahth.exe"]);
        for tag in ["Application", "uap3:Extension", "desktop:Extension"] {
            assert_eq!(attrs(tag, "EntryPoint"), ["Windows.FullTrustApplication"], "{tag}");
        }
        assert_eq!(attrs("rescap:Capability", "Name"), ["runFullTrust"]);
        assert_eq!(attrs("TargetDeviceFamily", "Name"), ["Windows.Desktop"]);
        assert_eq!(attrs("TargetDeviceFamily", "MinVersion"), ["10.0.17763.0"]);
        // The startup task is off until the program (or the user) turns it on.
        assert_eq!(attrs("desktop:StartupTask", "Enabled"), ["false"]);
        // Filled in by packaging/msix/make_layout.py.
        assert_eq!(attrs("Identity", "Name"), ["{{IDENTITY_NAME}}"]);
        assert_eq!(attrs("Identity", "Publisher"), ["{{PUBLISHER}}"]);
    }
}
