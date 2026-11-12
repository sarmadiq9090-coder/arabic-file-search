//! Which network the phone search may run on.
//!
//! The server of `crate::web` speaks plain HTTP, so it must only listen at
//! home, never on a café's, hotel's or university's network. A private
//! address (192.168.x.x…) says nothing about that; Windows does. Its Network
//! List Manager gives every network the PC joins an id that stays the same
//! each time, and a category: public, private, or a work domain. So:
//!
//! - by itself (at start-up, or when the network changes) the server runs
//!   only on the network the user turned it on at (the remembered id), and
//!   only while Windows marks that network private;
//! - when Windows cannot say which network an address is on, it does not
//!   start by itself;
//! - the user can allow the network they are on, in the window. A private
//!   one is remembered as home; any other is allowed until the program exits.
//!
//! [`decide`], [`plan`], [`allow_now`] and [`step`] are the whole policy,
//! without Windows calls. The Windows part ([`look`]) only reports each LAN
//! address with its network.

use std::net::Ipv4Addr;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

/// Windows' `NLM_NETWORK_CATEGORY`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    Public,
    Private,
    Domain,
}

impl Category {
    fn from_nlm(value: i32) -> Option<Category> {
        match value {
            0 => Some(Category::Public),
            1 => Some(Category::Private),
            2 => Some(Category::Domain),
            _ => None,
        }
    }
}

/// A network as Windows knows it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Network {
    /// The Network List Manager's network id, `{XXXXXXXX-XXXX-…}`: the same
    /// every time the PC joins this network, whatever addresses it gets.
    pub id: String,
    /// `None` when Windows did not say.
    pub category: Option<Category>,
    /// The name Windows shows for it (often the Wi-Fi name).
    pub name: String,
}

/// One of this PC's LAN addresses and the network it is on (`None`:
/// Windows could not tell).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Candidate {
    pub ip: Ipv4Addr,
    pub network: Option<Network>,
}

/// Who asks for the server to run on a network.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    /// The program itself: at start-up or after a network change.
    Auto,
    /// The user allowed this very network in the window, in this run.
    User,
}

/// Why the server does not run although the switch is on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// No LAN address at all.
    NoNetwork,
    /// Not the network the user turned it on at.
    NotHome,
    /// The remembered network, but Windows no longer marks it private.
    NotPrivate,
    /// Windows could not say which network this is, or its category.
    Unknown,
}

fn same_id(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// The rule: may the server run on the network with this `category` and id
/// `current`, when `remembered` is the id of the network the user turned it
/// on at?
pub fn decide(category: Option<Category>, remembered: Option<&str>, current: Option<&str>, choice: Choice) -> Result<(), Refusal> {
    if choice == Choice::User {
        return Ok(());
    }
    let (Some(category), Some(current)) = (category, current) else { return Err(Refusal::Unknown) };
    if !remembered.is_some_and(|r| same_id(r, current)) {
        return Err(Refusal::NotHome);
    }
    if category != Category::Private {
        return Err(Refusal::NotPrivate);
    }
    Ok(())
}

/// What the user allowed in this run of the program; forgotten at exit.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum Session {
    #[default]
    Nothing,
    /// The network with this id.
    Network(String),
    /// These addresses, while Windows cannot tell their network.
    Addresses(Vec<Ipv4Addr>),
}

impl Session {
    fn choice(&self, c: &Candidate) -> Choice {
        let allowed = match (self, &c.network) {
            (Session::Network(id), Some(n)) => same_id(id, &n.id),
            (Session::Addresses(ips), None) => ips.contains(&c.ip),
            _ => false,
        };
        if allowed { Choice::User } else { Choice::Auto }
    }
}

/// The addresses to listen on; when there are none, why.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Plan {
    pub bind: Vec<Ipv4Addr>,
    pub refusal: Option<Refusal>,
}

/// Applies [`decide`] to every LAN address: only those on an allowed
/// network are listened on (a VPN or a second network is left out).
pub fn plan(candidates: &[Candidate], remembered: Option<&str>, session: &Session) -> Plan {
    let mut bind = Vec::new();
    let mut refusals = Vec::new();
    for c in candidates {
        let (category, id) = (c.network.as_ref().and_then(|n| n.category), c.network.as_ref().map(|n| n.id.as_str()));
        match decide(category, remembered, id, session.choice(c)) {
            Ok(()) => bind.push(c.ip),
            Err(r) => refusals.push(r),
        }
    }
    let refusal = if !bind.is_empty() {
        None
    } else if candidates.is_empty() {
        Some(Refusal::NoNetwork)
    } else {
        // The most useful reason first.
        [Refusal::NotPrivate, Refusal::NotHome, Refusal::Unknown].into_iter().find(|r| refusals.contains(r))
    };
    Plan { bind, refusal }
}

/// The user ticked the switch or pressed "Allow this network": the id to
/// remember as home (only a network Windows marks private) and what is
/// allowed for this run.
pub fn allow_now(candidates: &[Candidate]) -> (Option<String>, Session) {
    let known = || candidates.iter().filter_map(|c| c.network.as_ref());
    let chosen = known().find(|n| n.category == Some(Category::Private)).or_else(|| known().next());
    match chosen {
        Some(n) => ((n.category == Some(Category::Private)).then(|| n.id.clone()), Session::Network(n.id.clone())),
        None => (None, Session::Addresses(candidates.iter().map(|c| c.ip).collect())),
    }
}

/// What to do with the listeners after a look at the network.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Step {
    Keep,
    Stop,
    /// (Re)start on exactly these LAN addresses.
    Listen(Vec<Ipv4Addr>),
}

/// `listening`: the LAN addresses the server listens on now (`None`: it is
/// stopped).
pub fn step(listening: Option<&[Ipv4Addr]>, plan: &Plan) -> Step {
    match listening {
        None if plan.bind.is_empty() => Step::Keep,
        Some(_) if plan.bind.is_empty() => Step::Stop,
        Some(now) if now == plan.bind.as_slice() => Step::Keep,
        _ => Step::Listen(plan.bind.clone()),
    }
}

/// The same addresses with no network known: what is left when Windows
/// does not answer.
pub fn without_networks(ips: &[Ipv4Addr]) -> Vec<Candidate> {
    ips.iter().map(|&ip| Candidate { ip, network: None }).collect()
}

// ---------------------------------------------------------------- matching

/// A network adapter's id (`{GUID}`) and its IPv4 addresses.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Adapter {
    pub id: String,
    pub ips: Vec<Ipv4Addr>,
}

/// A connected adapter (`{GUID}`) and the network it is connected to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Connection {
    pub adapter: String,
    pub network: Network,
}

/// The network of each address: that of the one adapter holding it. An
/// address on no adapter, on several, or on an adapter Windows lists no
/// network for, has none.
pub fn match_networks(ips: &[Ipv4Addr], adapters: &[Adapter], connections: &[Connection]) -> Vec<Candidate> {
    ips.iter()
        .map(|&ip| {
            let mut holders = adapters.iter().filter(|a| a.ips.contains(&ip));
            let network = match (holders.next(), holders.next()) {
                (Some(a), None) => connections.iter().find(|c| same_id(&c.adapter, &a.id)).map(|c| c.network.clone()),
                _ => None,
            };
            Candidate { ip, network }
        })
        .collect()
}

// ---------------------------------------------------------------- Windows

/// The Windows calls. Everything but the imported functions is compiled
/// (and type-checked) on every system; elsewhere the calls just fail.
#[cfg_attr(not(windows), allow(dead_code))]
mod sys {
    use super::{Adapter, Category, Connection, Network};
    use std::ffi::c_void;
    use std::net::Ipv4Addr;
    use std::ptr::null_mut;

    type Hresult = i32;

    #[repr(C)]
    #[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
    pub struct Guid {
        pub d1: u32,
        pub d2: u16,
        pub d3: u16,
        pub d4: [u8; 8],
    }

    impl Guid {
        /// `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}`, as Windows writes it.
        pub fn text(&self) -> String {
            let d = &self.d4;
            format!(
                "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
                self.d1, self.d2, self.d3, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
            )
        }
    }

    const fn nlm_guid(d1: u32) -> Guid {
        Guid { d1, d2: 0x570F, d3: 0x4A9B, d4: [0x8D, 0x69, 0x19, 0x9F, 0xDB, 0xA5, 0x72, 0x3B] }
    }
    /// `CLSID_NetworkListManager`.
    pub const CLSID_NETWORK_LIST_MANAGER: Guid = nlm_guid(0xDCB0_0C01);
    /// `IID_INetworkListManager`.
    pub const IID_NETWORK_LIST_MANAGER: Guid = nlm_guid(0xDCB0_0000);

    // The interfaces of netlistmgr.h, in vtable order. All derive from
    // IDispatch (IUnknown's 3 methods, then 4); methods never called here
    // are plain pointer-sized slots.
    #[repr(C)]
    #[allow(dead_code)] // layout: most slots are never read
    pub struct DispatchVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        dispatch: [usize; 4],
    }

    /// `INetworkListManager`.
    #[repr(C)]
    #[allow(dead_code)] // layout: most slots are never read
    pub struct ManagerVtbl {
        base: DispatchVtbl,
        get_networks: usize,
        get_network: usize,
        pub get_network_connections: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult,
    }

    /// `IEnumNetworkConnections`.
    #[repr(C)]
    #[allow(dead_code)] // layout: most slots are never read
    pub struct EnumConnectionsVtbl {
        base: DispatchVtbl,
        new_enum: usize,
        pub next: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void, *mut u32) -> Hresult,
    }

    /// `INetworkConnection`.
    #[repr(C)]
    #[allow(dead_code)] // layout: most slots are never read
    pub struct ConnectionVtbl {
        base: DispatchVtbl,
        pub get_network: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult,
        is_connected_to_internet: usize,
        is_connected: usize,
        get_connectivity: usize,
        get_connection_id: usize,
        pub get_adapter_id: unsafe extern "system" fn(*mut c_void, *mut Guid) -> Hresult,
    }

    /// `INetwork`.
    #[repr(C)]
    #[allow(dead_code)] // layout: most slots are never read
    pub struct NetworkVtbl {
        base: DispatchVtbl,
        pub get_name: unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> Hresult,
        set_name: usize,
        get_description: usize,
        set_description: usize,
        pub get_network_id: unsafe extern "system" fn(*mut c_void, *mut Guid) -> Hresult,
        get_domain_type: usize,
        get_network_connections: usize,
        get_time_created_and_connected: usize,
        is_connected_to_internet: usize,
        is_connected: usize,
        get_connectivity: usize,
        pub get_category: unsafe extern "system" fn(*mut c_void, *mut i32) -> Hresult,
    }

    /// `IP_ADDR_STRING` (iptypes.h).
    #[repr(C)]
    #[allow(dead_code)] // layout: most slots are never read
    pub struct IpAddrString {
        pub next: *mut IpAddrString,
        pub address: [u8; 16],
        mask: [u8; 16],
        context: u32,
    }

    /// The start of `IP_ADAPTER_INFO` (iptypes.h); the fields after the
    /// address list are never read.
    #[repr(C)]
    #[allow(dead_code)] // layout: most slots are never read
    pub struct AdapterInfo {
        pub next: *mut AdapterInfo,
        combo_index: u32,
        pub name: [u8; 260],
        description: [u8; 132],
        address_length: u32,
        address: [u8; 8],
        index: u32,
        kind: u32,
        dhcp_enabled: u32,
        current_ip: *mut IpAddrString,
        pub ip_list: IpAddrString,
    }

    #[cfg(windows)]
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> Hresult;
        fn CoUninitialize();
        fn CoCreateInstance(clsid: *const Guid, outer: *mut c_void, context: u32, iid: *const Guid, out: *mut *mut c_void) -> Hresult;
    }

    #[cfg(windows)]
    #[link(name = "oleaut32")]
    unsafe extern "system" {
        fn SysFreeString(s: *mut u16);
    }

    #[cfg(windows)]
    #[link(name = "iphlpapi")]
    unsafe extern "system" {
        fn GetAdaptersInfo(info: *mut AdapterInfo, size: *mut u32) -> u32;
    }

    // Not Windows: the same functions, always failing.
    #[cfg(not(windows))]
    #[allow(non_snake_case)]
    unsafe fn CoInitializeEx(_: *mut c_void, _: u32) -> Hresult {
        -1
    }
    #[cfg(not(windows))]
    #[allow(non_snake_case)]
    unsafe fn CoUninitialize() {}
    #[cfg(not(windows))]
    #[allow(non_snake_case)]
    unsafe fn CoCreateInstance(_: *const Guid, _: *mut c_void, _: u32, _: *const Guid, _: *mut *mut c_void) -> Hresult {
        -1
    }
    #[cfg(not(windows))]
    #[allow(non_snake_case)]
    unsafe fn SysFreeString(_: *mut u16) {}
    #[cfg(not(windows))]
    #[allow(non_snake_case)]
    unsafe fn GetAdaptersInfo(_: *mut AdapterInfo, _: *mut u32) -> u32 {
        1
    }

    const COINIT_MULTITHREADED: u32 = 0;
    /// `CLSCTX_INPROC_SERVER | CLSCTX_INPROC_HANDLER | CLSCTX_LOCAL_SERVER`.
    const CLSCTX_ALL: u32 = 0x1 | 0x2 | 0x4;
    const ERROR_BUFFER_OVERFLOW: u32 = 111;

    /// Ends COM on this thread if `CoInitializeEx` started it here.
    struct ComInit(bool);

    impl Drop for ComInit {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: paired with a successful CoInitializeEx on this thread.
                unsafe { CoUninitialize() };
            }
        }
    }

    /// An owned COM interface pointer, released when dropped.
    struct Com(*mut c_void);

    impl Com {
        /// SAFETY: the pointer must be a live interface whose vtable starts
        /// with the slots of `T`.
        unsafe fn vtbl<T>(&self) -> &T {
            unsafe { &**(self.0 as *const *const T) }
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: every interface here starts with IUnknown, and
                // this guard owns one reference.
                unsafe { (self.vtbl::<DispatchVtbl>().release)(self.0) };
            }
        }
    }

    /// The text before the first NUL.
    pub fn c_text(bytes: &[u8]) -> &str {
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        std::str::from_utf8(&bytes[..end]).unwrap_or("")
    }

    /// Reads a BSTR (at most 256 characters); the caller frees it.
    ///
    /// SAFETY: `s` must point to a NUL-terminated UTF-16 string.
    unsafe fn wide_text(s: *const u16) -> String {
        let mut units = Vec::new();
        for i in 0..256 {
            let u = unsafe { *s.add(i) };
            if u == 0 {
                break;
            }
            units.push(u);
        }
        String::from_utf16_lossy(&units)
    }

    /// Every adapter with its IPv4 addresses (`GetAdaptersInfo`). Empty on
    /// failure.
    pub fn adapters() -> Vec<Adapter> {
        let mut out = Vec::new();
        let mut size: u32 = 0;
        // SAFETY: a size query; nothing is written but `size`.
        let mut rc = unsafe { GetAdaptersInfo(null_mut(), &mut size) };
        // u64 units keep the buffer aligned for the structures.
        let mut buf: Vec<u64> = Vec::new();
        for _ in 0..3 {
            if rc != ERROR_BUFFER_OVERFLOW || size == 0 {
                break;
            }
            buf = vec![0u64; (size as usize).div_ceil(8) + 1];
            // SAFETY: the buffer holds at least `size` bytes.
            rc = unsafe { GetAdaptersInfo(buf.as_mut_ptr().cast(), &mut size) };
        }
        if rc != 0 || buf.is_empty() {
            return out;
        }
        let mut p = buf.as_ptr() as *const AdapterInfo;
        for _ in 0..256 {
            if p.is_null() {
                break;
            }
            // SAFETY: Windows filled the buffer with a list of
            // IP_ADAPTER_INFO linked by `next`, all inside the buffer,
            // which lives until this function returns.
            let info = unsafe { &*p };
            let mut ips = Vec::new();
            let mut a: *const IpAddrString = &info.ip_list;
            for _ in 0..64 {
                if a.is_null() {
                    break;
                }
                // SAFETY: as above, for the adapter's address list.
                let addr = unsafe { &*a };
                if let Ok(ip) = c_text(&addr.address).parse::<Ipv4Addr>()
                    && !ip.is_unspecified()
                {
                    ips.push(ip);
                }
                a = addr.next;
            }
            out.push(Adapter { id: c_text(&info.name).to_string(), ips });
            p = info.next;
        }
        out
    }

    /// Every connected adapter with its network, from the Network List
    /// Manager (`INetworkListManager::GetNetworkConnections`). `None` when
    /// Windows does not answer. Call it on a thread of its own: it starts
    /// COM there (and ends it).
    pub fn connections() -> Option<Vec<Connection>> {
        // SAFETY: plain COM start-up on this thread; undone by ComInit.
        let hr = unsafe { CoInitializeEx(null_mut(), COINIT_MULTITHREADED) };
        // S_OK and S_FALSE are both undone; RPC_E_CHANGED_MODE (the thread
        // already uses COM another way) still lets us call it.
        let _init = ComInit(hr >= 0);
        let mut p: *mut c_void = null_mut();
        // SAFETY: out-pointer for an INetworkListManager owned by `manager`.
        let hr = unsafe { CoCreateInstance(&CLSID_NETWORK_LIST_MANAGER, null_mut(), CLSCTX_ALL, &IID_NETWORK_LIST_MANAGER, &mut p) };
        if hr < 0 || p.is_null() {
            return None;
        }
        let manager = Com(p);
        let mut p: *mut c_void = null_mut();
        // SAFETY: `manager` is an INetworkListManager; the out-pointer
        // receives an IEnumNetworkConnections owned by `list`.
        let hr = unsafe { (manager.vtbl::<ManagerVtbl>().get_network_connections)(manager.0, &mut p) };
        if hr < 0 || p.is_null() {
            return None;
        }
        let list = Com(p);
        let mut out = Vec::new();
        for _ in 0..64 {
            let (mut p, mut got): (*mut c_void, u32) = (null_mut(), 0);
            // SAFETY: asks for one INetworkConnection, owned by `connection`.
            let hr = unsafe { (list.vtbl::<EnumConnectionsVtbl>().next)(list.0, 1, &mut p, &mut got) };
            // S_FALSE: the end of the list.
            if hr != 0 || got == 0 || p.is_null() {
                break;
            }
            let connection = Com(p);
            // SAFETY: `connection` is an INetworkConnection.
            let v = unsafe { connection.vtbl::<ConnectionVtbl>() };
            let mut adapter = Guid::default();
            // SAFETY: out-pointer to a GUID.
            if unsafe { (v.get_adapter_id)(connection.0, &mut adapter) } < 0 {
                continue;
            }
            let mut p: *mut c_void = null_mut();
            // SAFETY: the out-pointer receives an INetwork owned by `network`.
            if unsafe { (v.get_network)(connection.0, &mut p) } < 0 || p.is_null() {
                continue;
            }
            let network = Com(p);
            // SAFETY: `network` is an INetwork.
            let v = unsafe { network.vtbl::<NetworkVtbl>() };
            let mut id = Guid::default();
            // SAFETY: out-pointer to a GUID.
            if unsafe { (v.get_network_id)(network.0, &mut id) } < 0 {
                continue;
            }
            let mut category: i32 = -1;
            // SAFETY: out-pointer to an NLM_NETWORK_CATEGORY (an int).
            let category = if unsafe { (v.get_category)(network.0, &mut category) } >= 0 { Category::from_nlm(category) } else { None };
            let mut s: *mut u16 = null_mut();
            // SAFETY: the out-pointer receives a BSTR, read and then freed.
            let name = if unsafe { (v.get_name)(network.0, &mut s) } >= 0 && !s.is_null() {
                let name = unsafe { wide_text(s) };
                unsafe { SysFreeString(s) };
                name
            } else {
                String::new()
            };
            out.push(Connection { adapter: adapter.text(), network: Network { id: id.text(), category, name } });
        }
        Some(out)
    }
}

/// A look at the network: this PC's LAN addresses, each with the network
/// Windows says it is on. Blocking (it asks a Windows service); call it on
/// a thread of its own.
pub fn look() -> Vec<Candidate> {
    let ips = super::lan_addresses();
    if ips.is_empty() {
        return Vec::new();
    }
    match sys::connections() {
        Some(connections) => match_networks(&ips, &sys::adapters(), &connections),
        None => without_networks(&ips),
    }
}

/// [`look`] on a short-lived thread, waiting at most `timeout`; after that
/// the addresses come back with no network known (so nothing starts by
/// itself).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn look_within(timeout: Duration) -> Vec<Candidate> {
    let (tx, rx) = channel();
    let spawned = std::thread::Builder::new().name("web-net-look".into()).spawn(move || {
        let _ = tx.send(look());
    });
    match spawned.ok().and_then(|_| rx.recv_timeout(timeout).ok()) {
        Some(seen) => seen,
        None => without_networks(&super::lan_addresses()),
    }
}

/// Looks at the network every [`Watcher::EVERY`] on its own thread, so the
/// window never waits for Windows. The thread ends when this is dropped.
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Watcher {
    wake: Sender<()>,
    seen: Receiver<Vec<Candidate>>,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl Watcher {
    pub const EVERY: Duration = Duration::from_secs(5);

    pub fn start() -> Option<Watcher> {
        Watcher::start_with(look, Watcher::EVERY)
    }

    fn start_with(look: impl Fn() -> Vec<Candidate> + Send + 'static, every: Duration) -> Option<Watcher> {
        let (wake, woken) = channel::<()>();
        let (tx, seen) = channel();
        std::thread::Builder::new()
            .name("web-net".into())
            .spawn(move || {
                loop {
                    if tx.send(look()).is_err() {
                        break;
                    }
                    // Sleeps until the next look, a call for one now, or
                    // the end (the Watcher was dropped).
                    if woken.recv_timeout(every) == Err(RecvTimeoutError::Disconnected) {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Watcher { wake, seen })
    }

    /// The newest look since the last call, if any.
    pub fn latest(&self) -> Option<Vec<Candidate>> {
        self.seen.try_iter().last()
    }

    /// Asks for a look right now.
    pub fn look_now(&self) {
        let _ = self.wake.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "{11111111-2222-3333-4444-555555555555}";
    const CAFE: &str = "{AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE}";

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn net(id: &str, category: Category) -> Network {
        Network { id: id.into(), category: Some(category), name: "x".into() }
    }

    fn on(addr: &str, id: &str, category: Category) -> Candidate {
        Candidate { ip: ip(addr), network: Some(net(id, category)) }
    }

    fn unknown(addr: &str) -> Candidate {
        Candidate { ip: ip(addr), network: None }
    }

    /// The bug: with the switch left on, the server started by itself on
    /// any network with a private address. By itself it may start only on
    /// the remembered network, and only while Windows marks it private.
    #[test]
    fn starts_by_itself_only_on_the_remembered_private_network() {
        use Category::*;
        fn auto(category: Option<Category>, remembered: Option<&str>, current: Option<&str>) -> Result<(), Refusal> {
            decide(category, remembered, current, Choice::Auto)
        }
        assert_eq!(auto(Some(Private), Some(HOME), Some(HOME)), Ok(()));
        // The id is compared whatever its letter case.
        assert_eq!(auto(Some(Private), Some(&HOME.to_lowercase()), Some(HOME)), Ok(()));
        // A café, a hotel, a university: another network, whatever Windows
        // calls it, even "private".
        for category in [Public, Private, Domain] {
            assert_eq!(auto(Some(category), Some(HOME), Some(CAFE)), Err(Refusal::NotHome), "{category:?}");
        }
        // Switched on by an older version: no network remembered yet.
        for category in [Public, Private, Domain] {
            assert_eq!(auto(Some(category), None, Some(CAFE)), Err(Refusal::NotHome), "{category:?}");
        }
        // Home, but Windows marks it public (or a work domain).
        assert_eq!(auto(Some(Public), Some(HOME), Some(HOME)), Err(Refusal::NotPrivate));
        assert_eq!(auto(Some(Domain), Some(HOME), Some(HOME)), Err(Refusal::NotPrivate));
        // Windows does not say: never by itself.
        assert_eq!(auto(None, Some(HOME), Some(HOME)), Err(Refusal::Unknown));
        assert_eq!(auto(Some(Private), Some(HOME), None), Err(Refusal::Unknown));
        assert_eq!(auto(None, None, None), Err(Refusal::Unknown));
    }

    #[test]
    fn the_user_can_allow_the_network_they_are_on() {
        for category in [None, Some(Category::Public), Some(Category::Private), Some(Category::Domain)] {
            for current in [None, Some(CAFE)] {
                assert_eq!(decide(category, Some(HOME), current, Choice::User), Ok(()));
            }
        }
    }

    #[test]
    fn plans_listen_only_on_the_home_network() {
        use Category::*;
        let nothing = Session::Nothing;
        // At home.
        let p = plan(&[on("192.168.1.7", HOME, Private)], Some(HOME), &nothing);
        assert_eq!(p, Plan { bind: vec![ip("192.168.1.7")], refusal: None });
        // The same address on a café's network: nothing, and the reason.
        let p = plan(&[on("192.168.1.7", CAFE, Public)], Some(HOME), &nothing);
        assert_eq!(p, Plan { bind: vec![], refusal: Some(Refusal::NotHome) });
        // Home plus a VPN: only the home address.
        let p = plan(&[on("192.168.1.7", HOME, Private), on("10.8.0.2", CAFE, Public)], Some(HOME), &nothing);
        assert_eq!(p, Plan { bind: vec![ip("192.168.1.7")], refusal: None });
        // No network at all; an unknown one; home turned public.
        assert_eq!(plan(&[], Some(HOME), &nothing).refusal, Some(Refusal::NoNetwork));
        assert_eq!(plan(&[unknown("192.168.1.7")], Some(HOME), &nothing), Plan { bind: vec![], refusal: Some(Refusal::Unknown) });
        assert_eq!(plan(&[on("192.168.1.7", HOME, Public)], Some(HOME), &nothing).refusal, Some(Refusal::NotPrivate));
        // The most useful reason wins.
        let p = plan(&[unknown("10.0.0.2"), on("192.168.1.7", CAFE, Public), on("192.168.5.7", HOME, Public)], Some(HOME), &nothing);
        assert_eq!(p.refusal, Some(Refusal::NotPrivate));
    }

    #[test]
    fn allowing_remembers_only_private_networks() {
        use Category::*;
        // A private network becomes home and is allowed now.
        let seen = [on("192.168.1.7", HOME, Private)];
        let (home, session) = allow_now(&seen);
        assert_eq!((home.as_deref(), &session), (Some(HOME), &Session::Network(HOME.into())));
        assert_eq!(plan(&seen, home.as_deref(), &Session::Nothing).bind, [ip("192.168.1.7")], "also at the next start");

        // A public one is allowed for this run only: home stays as it was.
        let seen = [on("192.168.1.7", CAFE, Public)];
        let (home, session) = allow_now(&seen);
        assert_eq!((home, &session), (None, &Session::Network(CAFE.into())));
        assert_eq!(plan(&seen, Some(HOME), &session).bind, [ip("192.168.1.7")]);
        assert!(plan(&seen, Some(HOME), &Session::Nothing).bind.is_empty(), "not at the next start");
        // ... and not on the next network either.
        assert!(plan(&[on("192.168.1.7", "{other}", Public)], Some(HOME), &session).bind.is_empty());

        // With a VPN beside the private network, the private one is meant.
        let seen = [on("10.8.0.2", CAFE, Public), on("192.168.1.7", HOME, Private)];
        let (home, session) = allow_now(&seen);
        assert_eq!(home.as_deref(), Some(HOME));
        assert_eq!(plan(&seen, home.as_deref(), &session).bind, [ip("192.168.1.7")]);

        // Windows does not say: these addresses, this run, nothing remembered.
        let seen = [unknown("192.168.1.7")];
        let (home, session) = allow_now(&seen);
        assert_eq!((home, &session), (None, &Session::Addresses(vec![ip("192.168.1.7")])));
        assert_eq!(plan(&seen, None, &session).bind, [ip("192.168.1.7")]);
        // Another address, or a network Windows now names: back to the rule.
        assert!(plan(&[unknown("10.0.0.9")], None, &session).bind.is_empty());
        assert!(plan(&[on("192.168.1.7", CAFE, Public)], None, &session).bind.is_empty());
    }

    /// The server follows the network: it stops when the PC leaves home
    /// and comes back (on the new address) when it returns.
    #[test]
    fn steps_follow_the_network() {
        use Category::*;
        let home = [on("192.168.1.7", HOME, Private)];
        let at = |seen: &[Candidate]| plan(seen, Some(HOME), &Session::Nothing);
        assert_eq!(step(None, &at(&home)), Step::Listen(vec![ip("192.168.1.7")]));
        assert_eq!(step(Some(&[ip("192.168.1.7")]), &at(&home)), Step::Keep);
        // The same address on another network: stop.
        assert_eq!(step(Some(&[ip("192.168.1.7")]), &at(&[on("192.168.1.7", CAFE, Public)])), Step::Stop);
        assert_eq!(step(Some(&[ip("192.168.1.7")]), &at(&[])), Step::Stop);
        assert_eq!(step(None, &at(&[])), Step::Keep);
        // A new address at home: listen there.
        assert_eq!(step(Some(&[ip("192.168.1.7")]), &at(&[on("192.168.1.9", HOME, Private)])), Step::Listen(vec![ip("192.168.1.9")]));
    }

    #[test]
    fn addresses_get_the_network_of_their_adapter() {
        let wifi = "{0000AAAA-0000-0000-0000-000000000001}";
        let vpn = "{0000AAAA-0000-0000-0000-000000000002}";
        let adapters = [
            Adapter { id: wifi.to_lowercase(), ips: vec![ip("192.168.1.7")] },
            Adapter { id: vpn.into(), ips: vec![ip("10.8.0.2")] },
            Adapter { id: "{virtual}".into(), ips: vec![ip("172.20.0.1"), ip("192.168.99.1")] },
            Adapter { id: "{clone}".into(), ips: vec![ip("192.168.99.1")] },
        ];
        let connections = [
            Connection { adapter: wifi.into(), network: net(HOME, Category::Private) },
            Connection { adapter: vpn.into(), network: net(CAFE, Category::Public) },
            Connection { adapter: "{clone}".into(), network: net(HOME, Category::Private) },
        ];
        let ips = [ip("192.168.1.7"), ip("10.8.0.2"), ip("172.20.0.1"), ip("192.168.99.1"), ip("192.168.7.7")];
        let seen = match_networks(&ips, &adapters, &connections);
        assert_eq!(seen[0], on("192.168.1.7", HOME, Category::Private));
        assert_eq!(seen[0].network.as_ref().map(|n| n.name.as_str()), Some("x"));
        assert_eq!(seen[1], on("10.8.0.2", CAFE, Category::Public));
        // An adapter Windows lists no network for, an address on two
        // adapters, an address on none: unknown.
        assert_eq!(&seen[2..], [unknown("172.20.0.1"), unknown("192.168.99.1"), unknown("192.168.7.7")]);
        assert_eq!(without_networks(&ips[..1]), [unknown("192.168.1.7")]);
    }

    #[test]
    fn windows_values() {
        assert_eq!(Category::from_nlm(0), Some(Category::Public));
        assert_eq!(Category::from_nlm(1), Some(Category::Private));
        assert_eq!(Category::from_nlm(2), Some(Category::Domain));
        assert_eq!(Category::from_nlm(3), None);
        assert_eq!(Category::from_nlm(-1), None);
        assert_eq!(sys::CLSID_NETWORK_LIST_MANAGER.text(), "{DCB00C01-570F-4A9B-8D69-199FDBA5723B}");
        assert_eq!(sys::IID_NETWORK_LIST_MANAGER.text(), "{DCB00000-570F-4A9B-8D69-199FDBA5723B}");
        assert_eq!(sys::c_text(b"192.168.1.7\0junk"), "192.168.1.7");
        assert_eq!(sys::c_text(b"abc"), "abc");
        assert_eq!(sys::c_text(b"\0"), "");
    }

    /// The method slots match netlistmgr.h and the structures iptypes.h
    /// (checked against the `windows` and `winapi` crates' definitions).
    #[test]
    fn windows_layouts() {
        use std::mem::{offset_of, size_of};
        let slot = size_of::<usize>();
        assert_eq!(size_of::<sys::Guid>(), 16);
        assert_eq!(offset_of!(sys::ManagerVtbl, get_network_connections), 9 * slot);
        assert_eq!(offset_of!(sys::EnumConnectionsVtbl, next), 8 * slot);
        assert_eq!(offset_of!(sys::ConnectionVtbl, get_network), 7 * slot);
        assert_eq!(offset_of!(sys::ConnectionVtbl, get_adapter_id), 12 * slot);
        assert_eq!(offset_of!(sys::NetworkVtbl, get_name), 7 * slot);
        assert_eq!(offset_of!(sys::NetworkVtbl, get_network_id), 11 * slot);
        assert_eq!(offset_of!(sys::NetworkVtbl, get_category), 18 * slot);
        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(offset_of!(sys::AdapterInfo, name), 12);
            assert_eq!(offset_of!(sys::AdapterInfo, ip_list), 440);
            assert_eq!(offset_of!(sys::IpAddrString, address), 8);
            assert_eq!(size_of::<sys::IpAddrString>(), 48);
        }
    }

    /// Not on Windows: no network is known, so nothing starts by itself.
    #[cfg(not(windows))]
    #[test]
    fn a_look_without_windows_knows_no_network() {
        assert!(sys::connections().is_none());
        assert!(sys::adapters().is_empty());
        let seen = look_within(Duration::from_secs(5));
        assert!(seen.iter().all(|c| c.network.is_none()));
        assert!(plan(&seen, Some(HOME), &Session::Nothing).bind.is_empty());
    }

    #[test]
    fn the_watcher_reports_and_stops() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let looks = Arc::new(AtomicUsize::new(0));
        let counter = looks.clone();
        let watcher = Watcher::start_with(
            move || {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                vec![Candidate { ip: Ipv4Addr::new(192, 168, 1, n.min(200) as u8), network: None }]
            },
            Duration::from_secs(3600),
        )
        .unwrap();
        let wait_for = |n: usize| {
            for _ in 0..500 {
                if looks.load(Ordering::SeqCst) >= n {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        // One look at once, then none until asked (the pause is an hour).
        wait_for(1);
        assert_eq!(watcher.latest(), Some(vec![unknown("192.168.1.0")]));
        assert_eq!(watcher.latest(), None);
        watcher.look_now();
        wait_for(2);
        assert_eq!(watcher.latest(), Some(vec![unknown("192.168.1.1")]));
        // Dropping it ends the thread: no more looks.
        drop(watcher);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(looks.load(Ordering::SeqCst), 2);
    }
}
