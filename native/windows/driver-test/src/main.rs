// Acceptance tool for the extend.computer Windows indirect display driver.
//
// Run as the signed-in, non-elevated user (remote.sh driver-accept), through the
// shared client in native/windows/shared/display_driver.rs. It checks that:
//   1. GET_INFO reports the protocol and zero monitors; malformed requests are
//      rejected with the documented errors; a handle whose QoS hides the
//      caller's token (anonymous) is refused at open, and so is a
//      Low-integrity process of the same user;
//   2. monitors at sizes outside any common list (1366x1024, 1500x1000,
//      2388x1668, 2732x2048, no fallbacks) activate at exactly the requested
//      size, in the driver's surface and in the OS desktop layout; a duplicate
//      serial seed and a fifth monitor are refused;
//   3. another handle can neither query nor remove those monitors;
//   4. colours painted in a window on the monitor arrive in frames read through
//      the keyed mutex (counted only on S_OK), with their latency;
//   5. the newest frame is published after the client held the texture;
//   6. an abandoned keyed mutex makes the driver publish a new surface
//      generation, which the client detects and reopens;
//   7. a swap-chain reassign (topology off and on) does the same;
//   8. REMOVE and handle-close cleanup depart the monitors, in the driver and in
//      the OS device list;
//   9. monitors created and departed one after another (each with a swap chain
//      and a frame, by REMOVE and by handle close in turn) leave nothing
//      behind, and where this process may read the driver host's handle count
//      (an elevated run; remote.sh driver-accept prints it before and after
//      either way) it grows by no more than Windows' own per-swap-chain
//      handles (see `cycles` below);
//  10. handles closed right after ADD, while the monitor arrives or Windows
//      assigns its swap chain, neither crash the driver host nor leave a
//      monitor behind: back to back (synchronous ADDs, and overlapped ones
//      closed before they complete), then spaced out, one close 100-300 ms
//      after ADD every 3 s or so, alternating synchronous and overlapped, with
//      the driver host checked after each (a long-lived handle's GET_INFO, the
//      WUDFHost process, and the crash signals below), since a host that dies
//      in IddCx's watchdog does so a couple of seconds after the close;
//  11. the starting display topology is restored (saved with QDC_ALL_PATHS,
//      extended only if Windows did not activate the monitors by itself);
//  12. the driver host never crashed during the run: no UMDF host-problem
//      event (DriverFrameworks-UserMode/Operational 1009 or 4000), no critical
//      UMDF event in the System log (device offline, host terminated), no WER
//      report for a WUDF host and no new WudfHost dump.
//
// Windows draws nothing on any monitor while its display is powered off, so the
// tool wakes the display and keeps it on for the run.
//
// Other modes (`extend-computer-driver-test <mode> ...`):
//   open <luid-low> <luid-high> <name> [abandon]   opens a shared texture by
//       name and prints the result; with `abandon` it takes a frame and exits
//       holding it. The test uses that to abandon a keyed mutex.
//   monitor <width> <height> <seconds>   keeps one monitor for a while and
//       prints its texture name and render LUID, for manual checks such as
//       opening the texture from another logon session (denied).
//   churn [<count> [<min-ms> <max-ms>]]   check 10 (and 12) alone, with
//       <count> spaced closes (default 20) <min-ms> to <max-ms> (default 100
//       to 300) after ADD.
//   spaced [<count> [<min-ms> <max-ms>]]   only the spaced phase of check 10
//       (and 12), for a soak.
//   cycles [<count>]   check 9 (and 12) alone, with <count> cycles (default
//       20).
//   sustain [<seconds>]   one monitor in normal use: ADD, read frames for
//       <seconds> (default 30) while painting, REMOVE; then ADD and close the
//       handle after 10 s; and checks that the host survived and nothing was
//       left behind.
//   security   the open checks of check 1 alone (anonymous QoS, Low
//       integrity).
//   state   prints GET_INFO, the DISPLAY\XEC* monitors, the active display
//       paths and the driver host, changing nothing.
//   low-integrity-probe <path>   (used by check 1) prints this process's
//       integrity level and what it gets from the device interface at <path>.

#[cfg(windows)]
#[path = "../../shared/display_driver.rs"]
mod display_driver;

#[cfg(not(windows))]
fn main() {
    eprintln!("this acceptance tool only runs on Windows");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("open") => win::open_command(&args[1..]),
        Some("monitor") => win::monitor_command(&args[1..]),
        Some("churn") => win::churn_command(&args[1..]),
        Some("spaced") => win::spaced_command(&args[1..]),
        Some("cycles") => win::cycles_command(&args[1..]),
        Some("sustain") => win::sustain_command(&args[1..]),
        Some("state") => win::state_command(),
        Some("security") => win::security_command(),
        Some("low-integrity-probe") => win::low_integrity_probe(&args[1..]),
        _ => win::run(),
    };
    std::process::exit(code);
}

#[cfg(windows)]
mod win {
    use crate::display_driver::{
        self as dd, as_bytes, create_device, errors, win32_code, Acquire, AddMonitorIn,
        AddMonitorOut, Driver, Mode, QueryMonitorOut, SharedSurface, FLAG_FRAME, FLAG_PRESENT,
        FLAG_SURFACE,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::mem::size_of;
    use std::process::{Child, Command};
    use std::time::{Duration, Instant, SystemTime};
    use windows::core::{w, Result as WinResult, HSTRING, PCWSTR, PWSTR};
    use windows::Wdk::System::Threading::{
        NtQueryInformationProcess, ProcessCommandLineInformation,
    };
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        CM_Get_Device_ID_ListW, CM_Get_Device_ID_List_SizeW, CM_GETIDLIST_FILTER_ENUMERATOR,
        CM_GETIDLIST_FILTER_PRESENT, CR_SUCCESS,
    };
    use windows::Win32::Devices::Display::{
        GetDisplayConfigBufferSizes, QueryDisplayConfig, SetDisplayConfig, DISPLAYCONFIG_MODE_INFO,
        DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE, DISPLAYCONFIG_PATH_INFO, QDC_ALL_PATHS,
        QDC_ONLY_ACTIVE_PATHS, QUERY_DISPLAY_CONFIG_FLAGS, SDC_ALLOW_CHANGES, SDC_APPLY,
        SDC_TOPOLOGY_EXTEND, SDC_USE_SUPPLIED_DISPLAY_CONFIG,
    };
    use windows::Win32::Foundation::{
        CloseHandle, RtlNtStatusToDosError, SetHandleInformation, COLORREF,
        ERROR_INSUFFICIENT_BUFFER, ERROR_IO_PENDING, ERROR_SUCCESS, HANDLE, HANDLE_FLAGS,
        HANDLE_FLAG_INHERIT, HWND, LPARAM, LRESULT, LUID, NTSTATUS, POINT, RECT, UNICODE_STRING,
        WAIT_OBJECT_0, WPARAM,
    };
    use windows::Win32::Graphics::Direct3D11::{
        ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
        D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
    use windows::Win32::Graphics::Gdi::{
        CreateSolidBrush, DeleteObject, FillRect, GdiFlush, GetDC, MonitorFromPoint, ReleaseDC,
        DISPLAYCONFIG_PATH_ACTIVE, MONITOR_DEFAULTTONULL,
    };
    use windows::Win32::Security::{
        DuplicateTokenEx, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
        SecurityImpersonation, SetTokenInformation, TokenIntegrityLevel, TokenPrimary, PSID,
        SECURITY_ATTRIBUTES, SID_AND_ATTRIBUTES, TOKEN_ACCESS_MASK, TOKEN_ADJUST_DEFAULT,
        TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING, SECURITY_ANONYMOUS, SECURITY_IDENTIFICATION,
        SECURITY_SQOS_PRESENT,
    };
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::EventLog::{
        EvtClose, EvtNext, EvtQuery, EvtQueryChannelPath, EvtRender, EvtRenderEventXml, EVT_HANDLE,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Pipes::CreatePipe;
    use windows::Win32::System::Power::{
        SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
    };
    use windows::Win32::System::Threading::{
        CreateEventW, CreateProcessAsUserW, GetCurrentProcess, GetProcessHandleCount, OpenProcess,
        OpenProcessToken, WaitForSingleObject, CREATE_NO_WINDOW, PROCESS_INFORMATION,
        PROCESS_QUERY_LIMITED_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOW,
    };
    use windows::Win32::System::IO::{DeviceIoControl, OVERLAPPED};
    use windows::Win32::UI::HiDpi::{
        GetDpiForMonitor, SetProcessDpiAwareness, MDT_EFFECTIVE_DPI, PROCESS_PER_MONITOR_DPI_AWARE,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, PeekMessageW,
        RegisterClassW, SetWindowPos, TranslateMessage, HWND_TOPMOST, MSG, PM_REMOVE,
        SWP_NOACTIVATE, SWP_SHOWWINDOW, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        WS_EX_TOPMOST, WS_POPUP,
    };

    // Sizes outside any common-resolution list, so they only activate if the
    // driver offers exactly the requested mode. Distinct, so each is findable.
    const SIZES: [(u32, u32); 4] = [(1366, 1024), (1500, 1000), (2388, 1668), (2732, 2048)];
    // Stable seeds: the same test monitors map to the same Windows display
    // configuration entries on every run instead of piling up new ones.
    const SEED_BASE: u32 = 0xE7E0_0000;

    struct Report {
        pass: bool,
        lines: Vec<String>,
    }

    impl Report {
        fn check(&mut self, ok: bool, message: impl Into<String>) -> bool {
            self.pass &= ok;
            let tag = if ok { "PASS" } else { "FAIL" };
            self.lines.push(format!("[{tag}] {}", message.into()));
            ok
        }
        fn info(&mut self, message: impl Into<String>) {
            self.lines.push(format!("[info] {}", message.into()));
        }
    }

    fn sleep(ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }

    fn err_code<T>(r: &WinResult<T>) -> Option<u32> {
        r.as_ref().err().and_then(win32_code)
    }

    fn describe<T>(r: &WinResult<T>) -> String {
        match r {
            Ok(_) => "ok".into(),
            Err(e) => match win32_code(e) {
                Some(c) => format!("error {c}"),
                None => format!("{e}"),
            },
        }
    }

    fn mode_request(w: u32, h: u32, seed: u32) -> AddMonitorIn {
        AddMonitorIn::new(Mode::new(w, h, 60), seed)
    }

    const NO_SURFACE: &str = "Windows draws nothing on any monitor while its display is \
                              off or the session is locked";

    // Wakes the display with a zero-length mouse move and keeps it on until
    // `display_idle`, so Windows keeps presenting to the monitors.
    fn keep_display_on() {
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED);
            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dwFlags: MOUSEEVENTF_MOVE,
                        ..Default::default()
                    },
                },
            };
            SendInput(&[input], size_of::<INPUT>() as i32);
        }
        // A display coming out of power-off takes a moment to light up.
        sleep(1500);
    }

    fn display_idle() {
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS);
        }
    }

    // ---- display topology ----

    struct Config {
        paths: Vec<DISPLAYCONFIG_PATH_INFO>,
        modes: Vec<DISPLAYCONFIG_MODE_INFO>,
    }

    fn luid_key(l: LUID) -> i64 {
        (i64::from(l.HighPart) << 32) | i64::from(l.LowPart)
    }

    fn query_config(flags: QUERY_DISPLAY_CONFIG_FLAGS) -> anyhow::Result<Config> {
        loop {
            let (mut np, mut nm) = (0u32, 0u32);
            let e = unsafe { GetDisplayConfigBufferSizes(flags, &mut np, &mut nm) };
            anyhow::ensure!(e == ERROR_SUCCESS, "GetDisplayConfigBufferSizes: {}", e.0);
            let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
            let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
            let e = unsafe {
                QueryDisplayConfig(
                    flags,
                    &mut np,
                    paths.as_mut_ptr(),
                    &mut nm,
                    modes.as_mut_ptr(),
                    None,
                )
            };
            if e == ERROR_INSUFFICIENT_BUFFER {
                continue;
            }
            anyhow::ensure!(e == ERROR_SUCCESS, "QueryDisplayConfig: {}", e.0);
            paths.truncate(np as usize);
            modes.truncate(nm as usize);
            return Ok(Config { paths, modes });
        }
    }

    // (source adapter, source id, target adapter, target id) of each active path.
    fn active_set() -> Vec<(i64, u32, i64, u32)> {
        let Ok(c) = query_config(QDC_ONLY_ACTIVE_PATHS) else {
            return Vec::new();
        };
        let mut set: Vec<_> = c
            .paths
            .iter()
            .filter(|p| p.flags & DISPLAYCONFIG_PATH_ACTIVE != 0)
            .map(|p| {
                (
                    luid_key(p.sourceInfo.adapterId),
                    p.sourceInfo.id,
                    luid_key(p.targetInfo.adapterId),
                    p.targetInfo.id,
                )
            })
            .collect();
        set.sort();
        set
    }

    // The desktop rectangle (left, top, width, height) of a target's active path,
    // in physical pixels.
    fn target_rect(target: (LUID, u32)) -> Option<(i32, i32, u32, u32)> {
        let c = query_config(QDC_ONLY_ACTIVE_PATHS).ok()?;
        let path = c.paths.iter().find(|p| {
            luid_key(p.targetInfo.adapterId) == luid_key(target.0) && p.targetInfo.id == target.1
        })?;
        let mode = c
            .modes
            .get(unsafe { path.sourceInfo.Anonymous.modeInfoIdx } as usize)?;
        if mode.infoType != DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE {
            return None;
        }
        let source = unsafe { mode.Anonymous.sourceMode };
        Some((
            source.position.x,
            source.position.y,
            source.width,
            source.height,
        ))
    }

    fn wait_active(targets: &[(LUID, u32)], ms: u64) -> bool {
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            if targets.iter().all(|&t| target_rect(t).is_some()) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            sleep(250);
        }
    }

    fn extend() -> i32 {
        unsafe { SetDisplayConfig(None, None, SDC_APPLY | SDC_TOPOLOGY_EXTEND) }
    }

    // Applies a saved configuration exactly, falling back to letting Windows
    // adjust it, then to the saved active paths alone.
    fn apply(all: &Config, active: &Config) -> i32 {
        let exact = SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG;
        let mut last = 0;
        for (c, flags) in [
            (all, exact),
            (all, exact | SDC_ALLOW_CHANGES),
            (active, exact | SDC_ALLOW_CHANGES),
        ] {
            last = unsafe { SetDisplayConfig(Some(&c.paths), Some(&c.modes), flags) };
            if last == 0 {
                break;
            }
        }
        last
    }

    fn scale_at(x: i32, y: i32) -> Option<u32> {
        unsafe {
            let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL);
            if monitor.is_invalid() {
                return None;
            }
            let (mut dx, mut dy) = (0u32, 0u32);
            GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).ok()?;
            Some(dx * 100 / 96)
        }
    }

    // Present PnP monitors with our EDID manufacturer id, DISPLAY\XEC*.
    fn xec_monitors() -> usize {
        let flags = CM_GETIDLIST_FILTER_ENUMERATOR | CM_GETIDLIST_FILTER_PRESENT;
        unsafe {
            let mut len = 0u32;
            if CM_Get_Device_ID_List_SizeW(&mut len, w!("DISPLAY"), flags) != CR_SUCCESS {
                return 0;
            }
            let mut buf = vec![0u16; len as usize];
            if CM_Get_Device_ID_ListW(w!("DISPLAY"), &mut buf, flags) != CR_SUCCESS {
                return 0;
            }
            String::from_utf16_lossy(&buf)
                .split('\0')
                .filter(|id| id.to_ascii_uppercase().starts_with("DISPLAY\\XEC"))
                .count()
        }
    }

    fn wait_xec(count: usize, ms: u64) -> usize {
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            let n = xec_monitors();
            if n == count || Instant::now() >= deadline {
                return n;
            }
            sleep(250);
        }
    }

    // ---- the driver host ----

    // The device group in the driver host's command line (INF DeviceGroupId).
    const HOST_GROUP: &str = "ExtendComputerDisplayGroup";
    // Where UMDF writes a crashed host's minidump, WudfHost_*.mdmp.
    const HOST_DUMPS: &str = r"C:\ProgramData\Microsoft\WDF";

    // The WUDFHost.exe processes, and the one that hosts this driver when this
    // user may read its command line. A driver-host crash ends that process,
    // so its id changes when UMDF restarts the device.
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Hosts {
        ours: Option<u32>,
        all: Vec<u32>,
    }

    impl Hosts {
        fn now() -> Self {
            let mut hosts = Self {
                ours: None,
                all: Vec::new(),
            };
            unsafe {
                let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                    return hosts;
                };
                let mut entry = PROCESSENTRY32W {
                    dwSize: size_of::<PROCESSENTRY32W>() as u32,
                    ..Default::default()
                };
                let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
                while more {
                    let exe = &entry.szExeFile;
                    let len = exe.iter().position(|&c| c == 0).unwrap_or(exe.len());
                    if String::from_utf16_lossy(&exe[..len]).eq_ignore_ascii_case("WUDFHost.exe") {
                        let pid = entry.th32ProcessID;
                        hosts.all.push(pid);
                        if command_line(pid).is_some_and(|c| c.contains(HOST_GROUP)) {
                            hosts.ours = Some(pid);
                        }
                    }
                    more = Process32NextW(snapshot, &mut entry).is_ok();
                }
                let _ = CloseHandle(snapshot);
            }
            hosts.all.sort_unstable();
            hosts
        }

        // Whether the host that ran at `self` still runs: the same process for
        // this driver, or, when its command line is not readable, the same set
        // of WUDFHost processes.
        fn unchanged(&self, now: &Self) -> bool {
            match self.ours {
                Some(pid) => now.ours == Some(pid),
                None => now.all == self.all,
            }
        }

        fn describe(&self) -> String {
            match self.ours {
                Some(pid) => format!("driver host WUDFHost pid {pid}"),
                None => format!("WUDFHost pids {:?}", self.all),
            }
        }
    }

    // A process's command line, if this user may query it.
    fn command_line(pid: u32) -> Option<String> {
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            // A UNICODE_STRING followed by its characters; u64s keep it aligned.
            let mut buf = vec![0u64; 1024];
            let mut len = 0u32;
            let status = NtQueryInformationProcess(
                process,
                ProcessCommandLineInformation,
                buf.as_mut_ptr().cast(),
                (buf.len() * 8) as u32,
                &mut len,
            );
            let _ = CloseHandle(process);
            if status.0 < 0 {
                return None;
            }
            let text = &*(buf.as_ptr() as *const UNICODE_STRING);
            if text.Buffer.is_null() {
                return None;
            }
            let chars = std::slice::from_raw_parts(text.Buffer.0, usize::from(text.Length) / 2);
            Some(String::from_utf16_lossy(chars))
        }
    }

    // Records when the run started and which host dumps existed, to report
    // every sign of a UMDF driver-host crash since.
    struct CrashWatch {
        start: Instant,
        dumps: BTreeSet<(String, Option<SystemTime>)>,
    }

    impl CrashWatch {
        fn new() -> Self {
            Self {
                start: Instant::now(),
                dumps: host_dumps(),
            }
        }

        // The UMDF host-problem events (1009: "has a problem and is being
        // terminated", 4000: a driver runtime failure), the critical UMDF
        // events in the System log (device offline after a host crash, host
        // terminated, restart limit reached), WER reports for a WUDF host and
        // new host dumps, since `new`. A log that cannot be read counts too,
        // since then a crash could not be ruled out.
        fn signals(&self) -> Vec<String> {
            let window = format!(
                "TimeCreated[timediff(@SystemTime) <= {}]",
                self.start.elapsed().as_millis() + 500
            );
            let queries = [
                (
                    "Microsoft-Windows-DriverFrameworks-UserMode/Operational",
                    "(EventID=1009 or EventID=4000)",
                    None,
                ),
                (
                    "System",
                    "Provider[@Name='Microsoft-Windows-DriverFrameworks-UserMode'] and Level=1",
                    None,
                ),
                (
                    "Application",
                    "Provider[@Name='Windows Error Reporting'] and EventID=1001",
                    Some("WUDF"),
                ),
            ];
            let mut out = Vec::new();
            for (channel, filter, needle) in queries {
                match events(channel, &format!("*[System[{filter} and {window}]]")) {
                    Ok(list) => out.extend(
                        list.iter()
                            .filter(|xml| needle.is_none_or(|n| xml.contains(n)))
                            .map(|xml| {
                                format!(
                                    "{channel} event {} at {}",
                                    xml_element(xml, "EventID"),
                                    xml_attr(xml, "SystemTime")
                                )
                            }),
                    ),
                    Err(e) => out.push(format!("cannot read the {channel} log: {e}")),
                }
            }
            for (name, _) in host_dumps().difference(&self.dumps) {
                out.push(format!("new host dump {HOST_DUMPS}\\{name}"));
            }
            out
        }
    }

    fn host_dumps() -> BTreeSet<(String, Option<SystemTime>)> {
        let Ok(dir) = std::fs::read_dir(HOST_DUMPS) else {
            return BTreeSet::new();
        };
        dir.flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let dump = name.starts_with("WudfHost") && name.ends_with(".mdmp");
                dump.then(|| (name, e.metadata().ok().and_then(|m| m.modified().ok())))
            })
            .collect()
    }

    // The XML of each event in `channel` that `query` (XPath) selects.
    fn events(channel: &str, query: &str) -> WinResult<Vec<String>> {
        unsafe {
            let results = EvtQuery(
                None,
                &HSTRING::from(channel),
                &HSTRING::from(query),
                EvtQueryChannelPath.0,
            )?;
            let mut xml = Vec::new();
            loop {
                let mut batch = [0isize; 16];
                let mut n = 0u32;
                // Fails with ERROR_NO_MORE_ITEMS at the end.
                if EvtNext(results, &mut batch, 1000, 0, &mut n).is_err() {
                    break;
                }
                for &raw in &batch[..n as usize] {
                    let event = EVT_HANDLE(raw);
                    xml.push(render_xml(event));
                    let _ = EvtClose(event);
                }
            }
            let _ = EvtClose(results);
            Ok(xml)
        }
    }

    fn render_xml(event: EVT_HANDLE) -> String {
        let (mut used, mut count) = (0u32, 0u32);
        unsafe {
            let _ = EvtRender(
                None,
                event,
                EvtRenderEventXml.0,
                0,
                None,
                &mut used,
                &mut count,
            );
            let mut buf = vec![0u16; (used as usize).div_ceil(2)];
            let rendered = EvtRender(
                None,
                event,
                EvtRenderEventXml.0,
                (buf.len() * 2) as u32,
                Some(buf.as_mut_ptr().cast()),
                &mut used,
                &mut count,
            );
            if rendered.is_err() {
                return String::new();
            }
            String::from_utf16_lossy(&buf)
                .trim_end_matches('\0')
                .to_string()
        }
    }

    // The text of the first `<tag ...>text</tag>` in an event's XML.
    fn xml_element<'a>(xml: &'a str, tag: &str) -> &'a str {
        let text = xml.find(&format!("<{tag}")).and_then(|at| {
            let rest = &xml[at..];
            let start = rest.find('>')? + 1;
            let len = rest[start..].find('<')?;
            Some(&rest[start..start + len])
        });
        text.unwrap_or("?")
    }

    // The value of the first `name='value'` attribute in an event's XML.
    fn xml_attr<'a>(xml: &'a str, name: &str) -> &'a str {
        let value = xml.find(&format!("{name}=")).and_then(|at| {
            let rest = &xml[at + name.len() + 1..];
            let quote = rest.chars().next()?;
            let rest = &rest[1..];
            Some(&rest[..rest.find(quote)?])
        });
        value.unwrap_or("?")
    }

    // Records check 12: no driver-host crash since `crash` started.
    fn no_crash(r: &mut Report, crash: &CrashWatch, what: &str) {
        let signals = crash.signals();
        r.check(
            signals.is_empty(),
            if signals.is_empty() {
                format!(
                    "{what}: no driver-host crash (no UMDF 1009/4000 or critical UMDF event, \
                     no WER WUDF report, no new host dump)"
                )
            } else {
                format!("{what}: driver-host crash signals: {}", signals.join("; "))
            },
        );
    }

    // ---- frames ----

    type Rect = (i32, i32, u32, u32); // left, top, width, height

    // A small topmost window on the test monitor whose colour the test sets, so
    // the desktop changes on demand and each frame's content is known. (The
    // cursor would also work, but another app may clip or capture it.)
    struct Painter {
        hwnd: HWND,
    }

    const PAINTER_SIZE: i32 = 256;

    unsafe extern "system" fn painter_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    impl Painter {
        fn new(rect: Rect) -> anyhow::Result<Self> {
            unsafe {
                let instance = GetModuleHandleW(None)?;
                let class = w!("ExtendDriverTestPainter");
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(painter_proc),
                    hInstance: instance.into(),
                    lpszClassName: class,
                    ..Default::default()
                };
                RegisterClassW(&wc);
                let hwnd = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    class,
                    w!("extend.computer driver test"),
                    WS_POPUP,
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    Some(instance.into()),
                    None,
                )?;
                let painter = Self { hwnd };
                painter.place(rect)?;
                Ok(painter)
            }
        }

        // Centres the window on `rect`, over the block the reader samples.
        fn place(&self, rect: Rect) -> anyhow::Result<()> {
            let (cx, cy) = (rect.0 + rect.2 as i32 / 2, rect.1 + rect.3 as i32 / 2);
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    Some(HWND_TOPMOST),
                    cx - PAINTER_SIZE / 2,
                    cy - PAINTER_SIZE / 2,
                    PAINTER_SIZE,
                    PAINTER_SIZE,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )?;
            }
            pump();
            Ok(())
        }

        // Fills the window with `rgb` (0xRRGGBB).
        fn paint(&self, rgb: u32) {
            let colorref = ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF);
            unsafe {
                let dc = GetDC(Some(self.hwnd));
                let brush = CreateSolidBrush(COLORREF(colorref));
                let all = RECT {
                    left: 0,
                    top: 0,
                    right: PAINTER_SIZE,
                    bottom: PAINTER_SIZE,
                };
                FillRect(dc, &all, brush);
                let _ = DeleteObject(brush.into());
                ReleaseDC(Some(self.hwnd), dc);
                let _ = GdiFlush();
            }
            pump();
        }
    }

    impl Drop for Painter {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
            pump();
        }
    }

    fn pump() {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    // Pure, distinct colours, 0xRRGGBB.
    fn colour(i: usize) -> u32 {
        const COLOURS: [u32; 6] = [0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0x00FFFF, 0xFF00FF];
        COLOURS[i % COLOURS.len()]
    }

    struct Reader {
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        surface: Option<SharedSurface>,
        staging: Option<(ID3D11Texture2D, u32)>,
    }

    impl Reader {
        fn new(q: &QueryMonitorOut) -> anyhow::Result<Self> {
            let (device, context) = create_device(q.render_luid())?;
            let mut reader = Self {
                device,
                context,
                surface: None,
                staging: None,
            };
            reader.refresh(q)?;
            Ok(reader)
        }

        fn refresh(&mut self, q: &QueryMonitorOut) -> anyhow::Result<bool> {
            Ok(SharedSurface::reopen_if_stale(
                &mut self.surface,
                &self.device,
                q,
            )?)
        }

        fn surface(&self) -> anyhow::Result<&SharedSurface> {
            self.surface
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("no shared surface open"))
        }

        // Acquires a frame; on S_OK copies it out, releases it and returns the
        // centre pixel as 0xRRGGBB.
        fn read(&mut self, timeout_ms: u32) -> anyhow::Result<(Acquire, Option<u32>)> {
            let surface = self.surface()?;
            let result = surface.acquire(timeout_ms);
            if result != Acquire::Frame {
                return Ok((result, None));
            }
            let desc = *surface.desc();
            let generation = surface.generation();
            let texture = surface.texture().clone();
            if self.staging.as_ref().map(|s| s.1) != Some(generation) {
                self.staging = Some((staging_texture(&self.device, &desc)?, generation));
            }
            let staging = &self.staging.as_ref().unwrap().0;
            unsafe { self.context.CopyResource(staging, &texture) };
            self.surface()?.release()?;
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            unsafe {
                self.context
                    .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
                let row = (mapped.pData as *const u8)
                    .add((desc.Height / 2) as usize * mapped.RowPitch as usize);
                let px = row.add((desc.Width / 2) as usize * 4); // B8G8R8A8
                let rgb =
                    (u32::from(*px.add(2)) << 16) | (u32::from(*px.add(1)) << 8) | u32::from(*px);
                self.context.Unmap(staging, 0);
                Ok((Acquire::Frame, Some(rgb)))
            }
        }

        // Reads frames until one shows `rgb`; returns how long that took.
        fn wait_colour(&mut self, rgb: u32, ms: u64) -> anyhow::Result<Option<Duration>> {
            let start = Instant::now();
            let deadline = start + Duration::from_millis(ms);
            while Instant::now() < deadline {
                match self.read(50)? {
                    (Acquire::Frame, Some(px)) if px == rgb => return Ok(Some(start.elapsed())),
                    (Acquire::Frame | Acquire::Timeout, _) => {}
                    (other, _) => anyhow::bail!("acquire returned {other:?}"),
                }
            }
            Ok(None)
        }
    }

    fn staging_texture(
        device: &ID3D11Device,
        desc: &D3D11_TEXTURE2D_DESC,
    ) -> anyhow::Result<ID3D11Texture2D> {
        let staging = D3D11_TEXTURE2D_DESC {
            Width: desc.Width,
            Height: desc.Height,
            MipLevels: 1,
            ArraySize: 1,
            Format: desc.Format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut texture = None;
        unsafe { device.CreateTexture2D(&staging, None, Some(&mut texture))? };
        texture.ok_or_else(|| anyhow::anyhow!("no staging texture"))
    }

    // Polls QUERY until `done` holds or `ms` pass, calling `nudge` between polls
    // so the desktop produces frames.
    fn wait_query(
        dev: &Driver,
        id: u32,
        ms: u64,
        mut nudge: impl FnMut(usize),
        done: impl Fn(&QueryMonitorOut) -> bool,
    ) -> anyhow::Result<QueryMonitorOut> {
        let deadline = Instant::now() + Duration::from_millis(ms);
        let mut i = 0;
        loop {
            let q = dev.query_monitor(id)?;
            if done(&q) || Instant::now() >= deadline {
                return Ok(q);
            }
            nudge(i);
            i += 1;
            sleep(100);
        }
    }

    // ---- the test ----

    pub fn run() -> i32 {
        unsafe {
            let _ = SetProcessDpiAwareness(PROCESS_PER_MONITOR_DPI_AWARE);
        }
        let mut r = Report {
            pass: true,
            lines: Vec::new(),
        };
        keep_display_on();
        let crash = CrashWatch::new();

        let saved = (
            query_config(QDC_ALL_PATHS),
            query_config(QDC_ONLY_ACTIVE_PATHS),
        );
        let start_active = active_set();
        let start_xec = xec_monitors();
        let mut touched_topology = false;
        r.info(format!("driver host at start: {}", Hosts::now().describe()));

        let result = test(&mut r, &saved, &mut touched_topology);
        if let Err(e) = &result {
            r.check(false, format!("error: {e:#}"));
        }

        // Restore the starting topology no matter what happened. Our monitors are
        // gone by now (every handle is closed), so the saved paths all exist.
        match &saved {
            (Ok(all), Ok(active)) => {
                if touched_topology || active_set() != start_active {
                    let code = apply(all, active);
                    r.info(format!("restored the saved display configuration: {code}"));
                }
                sleep(500);
                r.check(
                    active_set() == start_active,
                    format!(
                        "display topology restored ({} active paths, as at start)",
                        start_active.len()
                    ),
                );
            }
            _ => {
                r.check(false, "could not save the starting display configuration");
            }
        }
        r.info(format!(
            "DISPLAY\\XEC* monitors at start: {start_xec}, at end: {}",
            xec_monitors()
        ));
        r.info(format!("driver host at end: {}", Hosts::now().describe()));
        // 12. No driver-host crash anywhere in the run.
        no_crash(&mut r, &crash, "whole run");
        display_idle();

        println!("==== extend.computer driver acceptance ====");
        for l in &r.lines {
            println!("{l}");
        }
        println!("==== {} ====", if r.pass { "PASS" } else { "FAIL" });
        i32::from(!r.pass)
    }

    fn test(
        r: &mut Report,
        saved: &(anyhow::Result<Config>, anyhow::Result<Config>),
        touched_topology: &mut bool,
    ) -> anyhow::Result<()> {
        // 1. Open, GET_INFO, malformed requests.
        let dev = Driver::open()?;
        let info = dev.info()?;
        r.check(
            info.struct_size == 28 && info.protocol_version == dd::PROTOCOL_VERSION,
            format!(
                "GET_INFO: protocol v{} driver {} iddcx 0x{:x} max {} active {}",
                info.protocol_version,
                info.driver_version(),
                info.iddcx_version,
                info.max_monitors,
                info.active_monitors
            ),
        );
        r.check(
            info.max_monitors == dd::MAX_MONITORS,
            format!("adapter supports {} monitors", info.max_monitors),
        );
        r.check(
            info.active_monitors == 0,
            format!(
                "adapter starts with 0 monitors (saw {})",
                info.active_monitors
            ),
        );
        malformed_requests(r, &dev)?;
        impersonation(r)?;

        // 2. Four monitors at non-list sizes; duplicate seed and a fifth refused.
        let mut added: Vec<(AddMonitorOut, (u32, u32))> = Vec::new();
        for (i, &(w, h)) in SIZES.iter().enumerate() {
            let out = dev.add_monitor(&mode_request(w, h, SEED_BASE + i as u32))?;
            if i == 0 {
                let dup = dev.add_monitor(&mode_request(1500, 1000, SEED_BASE));
                r.check(
                    err_code(&dup) == Some(errors::ALREADY_EXISTS),
                    format!(
                        "ADD with an active monitor's serial seed refused: {}",
                        describe(&dup)
                    ),
                );
            }
            added.push((out, (w, h)));
        }
        r.check(
            added.iter().all(|(a, _)| a.monitor_id != 0),
            format!(
                "ADD x4 ids {:?}, targets {:?}",
                added.iter().map(|(a, _)| a.monitor_id).collect::<Vec<_>>(),
                added
                    .iter()
                    .map(|(a, _)| a.os_target_id)
                    .collect::<Vec<_>>()
            ),
        );
        let fifth = dev.add_monitor(&mode_request(1280, 800, SEED_BASE + 9));
        r.check(
            err_code(&fifth) == Some(errors::NO_SYSTEM_RESOURCES),
            format!("5th ADD refused: {}", describe(&fifth)),
        );
        r.check(
            wait_xec(4, 5000) == 4,
            format!("OS lists 4 DISPLAY\\XEC* monitors (saw {})", xec_monitors()),
        );

        let targets: Vec<(LUID, u32)> = added
            .iter()
            .map(|(a, _)| (a.os_adapter_luid(), a.os_target_id))
            .collect();
        if wait_active(&targets, 4000) {
            r.info("Windows activated the monitors by itself");
        } else {
            let code = extend();
            *touched_topology = true;
            r.info(format!("SetDisplayConfig(extend) = {code}"));
        }
        let active = wait_active(&targets, 8000);
        r.check(active, "all four monitors are active desktop targets");

        for (a, (w, h)) in &added {
            let q = wait_query(
                &dev,
                a.monitor_id,
                10_000,
                |_| {},
                |q| q.has(FLAG_SURFACE) && q.has(FLAG_FRAME),
            )?;
            let rect = target_rect((a.os_adapter_luid(), a.os_target_id));
            r.check(
                q.has(FLAG_PRESENT)
                    && q.has(FLAG_SURFACE)
                    && q.width == *w
                    && q.height == *h
                    && q.surface_generation >= 1
                    && q.dxgi_format != 0,
                format!(
                    "monitor {} surface {}x{} fmt {} gen {} (requested {w}x{h}) name {}{}",
                    a.monitor_id,
                    q.width,
                    q.height,
                    q.dxgi_format,
                    q.surface_generation,
                    q.texture_name(),
                    if q.has(FLAG_SURFACE) {
                        String::new()
                    } else {
                        format!(" (no surface after 10 s: {NO_SURFACE})")
                    }
                ),
            );
            r.check(
                rect.is_some_and(|(_, _, rw, rh)| rw == *w && rh == *h),
                format!(
                    "monitor {} desktop rect {rect:?} at {:?}% scale",
                    a.monitor_id,
                    rect.and_then(|(x, y, rw, rh)| scale_at(x + rw as i32 / 2, y + rh as i32 / 2))
                ),
            );
        }

        // 3. Another handle cannot touch them.
        let (first, _) = added[0];
        {
            let other = Driver::open()?;
            let q = other.query_monitor(first.monitor_id);
            let rm = other.remove_monitor(first.monitor_id);
            r.check(
                err_code(&q) == Some(errors::ACCESS_DENIED)
                    && err_code(&rm) == Some(errors::ACCESS_DENIED),
                format!(
                    "non-owner QUERY {} and REMOVE {} denied",
                    describe(&q),
                    describe(&rm)
                ),
            );
        }
        r.check(
            dev.info()?.active_monitors == 4,
            "closing a handle that owns nothing leaves the 4 monitors",
        );

        frames(r, &dev, first, saved, touched_topology)?;

        // 8. REMOVE one, close the handle with three left, check both views.
        dev.remove_monitor(first.monitor_id)?;
        let q = dev.query_monitor(first.monitor_id);
        r.check(
            dev.info()?.active_monitors == 3 && err_code(&q) == Some(errors::NOT_FOUND),
            format!("REMOVE departs the monitor (QUERY now {})", describe(&q)),
        );
        drop(dev);
        sleep(500);
        let fresh = Driver::open()?;
        let info = fresh.info()?;
        let gone = added[1..]
            .iter()
            .all(|(a, _)| err_code(&fresh.query_monitor(a.monitor_id)) == Some(errors::NOT_FOUND));
        r.check(
            info.active_monitors == 0 && gone,
            format!(
                "handle close departed the other 3 monitors (active {})",
                info.active_monitors
            ),
        );
        let left = wait_xec(0, 8000);
        r.check(
            left == 0,
            format!("no DISPLAY\\XEC* monitor left in the OS device list (saw {left})"),
        );
        drop(fresh);

        // 9. Monitors created and departed one after another leave the driver
        // host's handle count flat.
        cycles(r, 6, touched_topology)?;

        // 10. Handles closed during arrival and swap-chain assignment.
        churn(r, SPACED)
    }

    fn frames(
        r: &mut Report,
        dev: &Driver,
        first: AddMonitorOut,
        saved: &(anyhow::Result<Config>, anyhow::Result<Config>),
        touched_topology: &mut bool,
    ) -> anyhow::Result<()> {
        let id = first.monitor_id;
        let target = (first.os_adapter_luid(), first.os_target_id);
        let rect = target_rect(target)
            .ok_or_else(|| anyhow::anyhow!("monitor {id} has no desktop rect"))?;
        let painter = Painter::new(rect)?;
        let q = dev.query_monitor(id)?;
        anyhow::ensure!(
            q.has(FLAG_SURFACE),
            "monitor {id} has no surface to read: {NO_SURFACE}"
        );
        let mut reader = Reader::new(&q)?;
        let g1 = q.surface_generation;

        // 4. Each colour painted on the monitor arrives in a frame read through
        //    the keyed mutex (frames count only on S_OK).
        let n = 30;
        let mut latencies = Vec::new();
        for i in 0..n {
            painter.paint(colour(i));
            if let Some(latency) = reader.wait_colour(colour(i), 1000)? {
                latencies.push(latency.as_secs_f64() * 1000.0);
            }
        }
        let max = latencies.iter().cloned().fold(0.0, f64::max);
        let avg = latencies.iter().sum::<f64>() / latencies.len().max(1) as f64;
        r.check(
            latencies.len() == n,
            format!(
                "{}/{n} painted colours arrived in frames read on S_OK; paint-to-read \
                 latency avg {avg:.1} ms, max {max:.1} ms",
                latencies.len()
            ),
        );

        // 5. The newest frame survives the client holding the texture: colours
        //    painted meanwhile are not published, and the last one is the next
        //    frame after release, with no further desktop change.
        painter.paint(0x808080);
        reader.wait_colour(0x808080, 1000)?;
        painter.paint(0x404040);
        let held = hold_frame(&reader)?;
        let before = dev.query_monitor(id)?.frame_counter();
        painter.paint(0xFF0000);
        sleep(50);
        painter.paint(0x00FF00);
        sleep(150);
        let during = dev.query_monitor(id)?.frame_counter();
        held.release()?;
        let (next, px) = reader.read(200)?;
        r.check(
            during == before && next == Acquire::Frame && px == Some(0x00FF00),
            format!(
                "held texture: nothing published while held (counter {before}->{during}); \
                 the next frame after release is {next:?} showing {px:06x?} (want 00ff00)"
            ),
        );

        // 6. An abandoned keyed mutex: a child process takes a frame and exits
        //    holding it; the driver must publish a new generation.
        let q = dev.query_monitor(id)?;
        let mut child = spawn_abandon(&q)?;
        let mut i = 0;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            painter.paint(colour(i));
            i += 1;
            sleep(30);
        };
        r.check(
            status.code() == Some(0),
            format!("child process took a frame and died holding it ({status})"),
        );
        let q2 = wait_query(
            dev,
            id,
            5000,
            |i| painter.paint(colour(i)),
            |q| q.surface_generation > g1,
        )?;
        let stale = reader.surface()?.acquire(50);
        let reopened = reader.refresh(&q2)?;
        painter.paint(0x00FFFF);
        let after_abandon = reader.wait_colour(0x00FFFF, 2000)?;
        r.check(
            q2.surface_generation > g1
                && q2.has(FLAG_SURFACE)
                && q2.texture_name() != q.texture_name()
                && reopened
                && after_abandon.is_some(),
            format!(
                "abandoned mutex -> generation {g1}->{} ({}), old texture says {stale:?}, \
                 reopened and read the next colour: {}",
                q2.surface_generation,
                q2.texture_name(),
                after_abandon.is_some()
            ),
        );

        // 7. A swap-chain reassign: take the monitors off the desktop and back.
        let g2 = q2.surface_generation;
        if let (Ok(all), Ok(active)) = saved {
            let code = apply(all, active);
            *touched_topology = true;
            let off = wait_query(dev, id, 6000, |_| {}, |q| !q.has(FLAG_SURFACE))?;
            r.check(
                code == 0 && !off.has(FLAG_SURFACE),
                format!("topology without our monitors ({code}) withdrew the surface"),
            );
            r.info(format!("SetDisplayConfig(extend) = {}", extend()));
            wait_active(&[target], 8000);
            let rect = target_rect(target).unwrap_or(rect);
            painter.place(rect)?;
            let q3 = wait_query(
                dev,
                id,
                10_000,
                |i| painter.paint(colour(i)),
                |q| q.has(FLAG_SURFACE) && q.surface_generation > g2,
            )?;
            let stale = reader.surface()?.acquire(50);
            let reopened = reader.refresh(&q3)?;
            painter.paint(0xFF00FF);
            let after_reassign = reader.wait_colour(0xFF00FF, 2000)?;
            r.check(
                q3.surface_generation > g2
                    && reopened
                    && after_reassign.is_some()
                    && q3.width == q.width
                    && q3.height == q.height,
                format!(
                    "reassign -> generation {g2}->{} {}x{}, old texture says {stale:?}, \
                     reopened and read the next colour: {}",
                    q3.surface_generation,
                    q3.width,
                    q3.height,
                    after_reassign.is_some()
                ),
            );
        }
        Ok(())
    }

    // Acquires the frame already waiting (the last colour painted) and returns
    // the surface still held.
    fn hold_frame(reader: &Reader) -> anyhow::Result<&SharedSurface> {
        match reader.surface()?.acquire(1000) {
            Acquire::Frame => reader.surface(),
            other => anyhow::bail!("no frame to hold: {other:?}"),
        }
    }

    fn spawn_abandon(q: &QueryMonitorOut) -> anyhow::Result<Child> {
        Ok(Command::new(std::env::current_exe()?)
            .args([
                "open".to_string(),
                q.render_luid_low.to_string(),
                q.render_luid_high.to_string(),
                q.texture_name(),
                "abandon".to_string(),
            ])
            .spawn()?)
    }

    // Malformed requests must fail with the documented errors and create nothing.
    fn malformed_requests(r: &mut Report, dev: &Driver) -> anyhow::Result<()> {
        use errors::{INSUFFICIENT_BUFFER, INVALID_FUNCTION, INVALID_PARAMETER, NOT_FOUND};
        let good = mode_request(1366, 1024, SEED_BASE + 7);
        let bytes = as_bytes(&good).to_vec();
        let mut out = [0u8; 256];
        let mut cases: Vec<(String, WinResult<u32>, u32)> = Vec::new();

        // (name, how to break a good request, expected error)
        type Edit = (&'static str, fn(&mut AddMonitorIn), u32);
        let fields: [Edit; 9] = [
            ("StructSize 0", |q| q.struct_size = 0, INVALID_PARAMETER),
            ("Version 1", |q| q.version = 1, errors::REVISION_MISMATCH),
            (
                "width below min",
                |q| q.width = dd::MIN_DIMENSION - 1,
                INVALID_PARAMETER,
            ),
            (
                "width above max",
                |q| q.width = dd::MAX_DIMENSION + 1,
                INVALID_PARAMETER,
            ),
            ("height 0", |q| q.height = 0, INVALID_PARAMETER),
            ("refresh 0", |q| q.refresh_hz = 0, INVALID_PARAMETER),
            (
                "refresh above max",
                |q| q.refresh_hz = dd::MAX_REFRESH_HZ + 1,
                INVALID_PARAMETER,
            ),
            (
                "9 fallbacks",
                |q| q.fallback_mode_count = 9,
                INVALID_PARAMETER,
            ),
            (
                "invalid fallback",
                |q| q.fallback_mode_count = 1,
                INVALID_PARAMETER,
            ),
        ];
        for (name, edit, want) in fields {
            let mut req = good;
            edit(&mut req);
            let res = dev.ioctl(dd::IOCTL_ADD_MONITOR, as_bytes(&req), &mut out[..24]);
            cases.push((format!("ADD {name}"), res, want));
        }

        let id = as_bytes(&dd::MonitorIdIn::new(0xFFFF_FFFF)).to_vec();
        let unknown = (0x22 << 16) | (3 << 14) | (0x8FF << 2);
        let long = [bytes.clone(), vec![0]].concat();
        let raw: [(&str, u32, &[u8], usize, u32); 10] = [
            (
                "ADD 131-byte input",
                dd::IOCTL_ADD_MONITOR,
                &bytes[..131],
                24,
                INVALID_PARAMETER,
            ),
            (
                "ADD 133-byte input",
                dd::IOCTL_ADD_MONITOR,
                &long,
                24,
                INVALID_PARAMETER,
            ),
            (
                "ADD 23-byte output",
                dd::IOCTL_ADD_MONITOR,
                &bytes,
                23,
                INSUFFICIENT_BUFFER,
            ),
            (
                "GET_INFO 4-byte input",
                dd::IOCTL_GET_INFO,
                &[0; 4],
                28,
                INVALID_PARAMETER,
            ),
            (
                "GET_INFO 27-byte output",
                dd::IOCTL_GET_INFO,
                &[],
                27,
                INSUFFICIENT_BUFFER,
            ),
            (
                "QUERY unknown id",
                dd::IOCTL_QUERY_MONITOR,
                &id,
                172,
                NOT_FOUND,
            ),
            (
                "QUERY 7-byte input",
                dd::IOCTL_QUERY_MONITOR,
                &id[..7],
                172,
                INVALID_PARAMETER,
            ),
            (
                "QUERY 171-byte output",
                dd::IOCTL_QUERY_MONITOR,
                &id,
                171,
                INSUFFICIENT_BUFFER,
            ),
            (
                "REMOVE unknown id",
                dd::IOCTL_REMOVE_MONITOR,
                &id,
                0,
                NOT_FOUND,
            ),
            ("unknown IOCTL", unknown, &[], 0, INVALID_FUNCTION),
        ];
        for (name, code, input, out_len, want) in raw {
            let res = dev.ioctl(code, input, &mut out[..out_len]);
            cases.push((name.to_string(), res, want));
        }

        let wrong: Vec<String> = cases
            .iter()
            .filter(|(_, res, want)| err_code(res) != Some(*want))
            .map(|(name, res, want)| format!("{name}: {} (want {want})", describe(res)))
            .collect();
        let active = dev.info()?.active_monitors;
        r.check(
            wrong.is_empty() && active == 0,
            format!(
                "{} malformed requests rejected as documented, none created a monitor \
                 (active {active}){}",
                cases.len(),
                if wrong.is_empty() {
                    String::new()
                } else {
                    format!(": {}", wrong.join("; "))
                }
            ),
        );
        Ok(())
    }

    // The driver reads the caller's token at identification level: its
    // integrity level at open and its logon SID on ADD. A handle opened with
    // anonymous QoS hides the token, so the open must be refused (fail
    // closed); one opened without SQOS flags gets the system default level and
    // works. A Low-integrity process of this same user, as a sandboxed browser
    // or document viewer would be, must not get in at all.
    fn impersonation(r: &mut Report) -> anyhow::Result<()> {
        let path = dd::interface_list()?;
        let anonymous = add_on_raw_handle(
            &path,
            SECURITY_SQOS_PRESENT | SECURITY_ANONYMOUS,
            SEED_BASE + 8,
        );
        r.check(
            matches!(anonymous, RawAdd::OpenRefused(errors::ACCESS_DENIED)),
            format!("an anonymous-QoS handle is refused: {anonymous}"),
        );
        let default = add_on_raw_handle(&path, FILE_FLAGS_AND_ATTRIBUTES(0), SEED_BASE + 8);
        r.check(
            matches!(default, RawAdd::Add(Ok(_))),
            format!("a handle without SQOS flags works: {default}"),
        );
        match run_low_integrity(&path) {
            Ok(out) => {
                let low = out.contains("integrity 0x1000");
                let refused = out.contains("open: error 5");
                r.check(
                    low && refused,
                    format!(
                        "a Low-integrity process of this user cannot open the device ({})",
                        out.trim().replace("\r\n", "; ").replace('\n', "; ")
                    ),
                );
            }
            Err(e) => {
                r.check(false, format!("could not run a Low-integrity probe: {e:#}"));
            }
        }
        Ok(())
    }

    // How an ADD on a raw handle went: refused at open, or the ADD's result.
    enum RawAdd {
        OpenRefused(u32),
        Add(WinResult<u32>),
    }

    impl std::fmt::Display for RawAdd {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            match self {
                Self::OpenRefused(code) => write!(f, "open refused with error {code}"),
                Self::Add(res) => write!(f, "opened, ADD {}", describe(res)),
            }
        }
    }

    // Opens the interface at `path` with `flags`, sends one ADD, closes the
    // handle (which departs the monitor if one was created).
    fn add_on_raw_handle(path: &[u16], flags: FILE_FLAGS_AND_ATTRIBUTES, seed: u32) -> RawAdd {
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                0xC000_0000,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                flags,
                None,
            )
        };
        let handle = match handle {
            Ok(h) => h,
            Err(e) => return RawAdd::OpenRefused(win32_code(&e).unwrap_or(e.code().0 as u32)),
        };
        let req = mode_request(1366, 1024, seed);
        let mut out = AddMonitorOut::default();
        let mut returned = 0u32;
        let res = unsafe {
            DeviceIoControl(
                handle,
                dd::IOCTL_ADD_MONITOR,
                Some(as_bytes(&req).as_ptr().cast()),
                size_of::<AddMonitorIn>() as u32,
                Some((&mut out as *mut AddMonitorOut).cast()),
                size_of::<AddMonitorOut>() as u32,
                Some(&mut returned),
                None,
            )
        };
        unsafe {
            let _ = CloseHandle(handle);
        }
        sleep(300);
        RawAdd::Add(res.map(|()| returned))
    }

    // ---- integrity ----

    // S-1-16-4096, the Low mandatory level.
    const LOW_INTEGRITY_SID: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 16, 0x00, 0x10, 0, 0];

    // Runs `low-integrity-probe <path>` from this executable at Low integrity,
    // with this process's token otherwise unchanged, and returns its output.
    fn run_low_integrity(path: &[u16]) -> anyhow::Result<String> {
        let len = path.iter().position(|&c| c == 0).unwrap_or(path.len());
        let path = String::from_utf16_lossy(&path[..len]);
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ADJUST_DEFAULT | TOKEN_ASSIGN_PRIMARY,
                &mut token,
            )?;
            let mut low = HANDLE::default();
            let duplicated = DuplicateTokenEx(
                token,
                TOKEN_ACCESS_MASK(0),
                None,
                SecurityImpersonation,
                TokenPrimary,
                &mut low,
            );
            let _ = CloseHandle(token);
            duplicated?;
            let mut sid = LOW_INTEGRITY_SID;
            let label = TOKEN_MANDATORY_LABEL {
                Label: SID_AND_ATTRIBUTES {
                    Sid: PSID(sid.as_mut_ptr().cast()),
                    Attributes: 0x20, // SE_GROUP_INTEGRITY
                },
            };
            let lowered = SetTokenInformation(
                low,
                TokenIntegrityLevel,
                (&label as *const TOKEN_MANDATORY_LABEL).cast(),
                (size_of::<TOKEN_MANDATORY_LABEL>() + sid.len()) as u32,
            );
            if let Err(e) = lowered {
                let _ = CloseHandle(low);
                return Err(e.into());
            }

            // The child's output comes back through an inheritable pipe.
            let inherit = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: std::ptr::null_mut(),
                bInheritHandle: true.into(),
            };
            let (mut read, mut write) = (HANDLE::default(), HANDLE::default());
            CreatePipe(&mut read, &mut write, Some(&inherit), 0)?;
            SetHandleInformation(read, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0))?;
            let startup = STARTUPINFOW {
                cb: size_of::<STARTUPINFOW>() as u32,
                dwFlags: STARTF_USESTDHANDLES,
                hStdOutput: write,
                hStdError: write,
                ..Default::default()
            };
            let mut process = PROCESS_INFORMATION::default();
            let exe = std::env::current_exe()?;
            let mut command: Vec<u16> =
                format!("\"{}\" low-integrity-probe \"{path}\"", exe.display())
                    .encode_utf16()
                    .chain(Some(0))
                    .collect();
            let created = CreateProcessAsUserW(
                Some(low),
                PCWSTR::null(),
                Some(PWSTR(command.as_mut_ptr())),
                None,
                None,
                true,
                CREATE_NO_WINDOW,
                None,
                PCWSTR::null(),
                &startup,
                &mut process,
            );
            let _ = CloseHandle(write);
            let _ = CloseHandle(low);
            if let Err(e) = created {
                let _ = CloseHandle(read);
                return Err(anyhow::anyhow!("CreateProcessAsUserW: {e}"));
            }
            let mut out = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let mut n = 0u32;
                if ReadFile(read, Some(&mut buf), Some(&mut n), None).is_err() || n == 0 {
                    break;
                }
                out.extend_from_slice(&buf[..n as usize]);
            }
            let _ = CloseHandle(read);
            WaitForSingleObject(process.hProcess, 10_000);
            let _ = CloseHandle(process.hProcess);
            let _ = CloseHandle(process.hThread);
            Ok(String::from_utf8_lossy(&out).into_owned())
        }
    }

    // This process's integrity level (its mandatory label's RID).
    fn own_integrity() -> Option<u32> {
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
            let mut buf = vec![0u64; 16];
            let mut len = 0u32;
            let read = GetTokenInformation(
                token,
                TokenIntegrityLevel,
                Some(buf.as_mut_ptr().cast()),
                (buf.len() * 8) as u32,
                &mut len,
            );
            let _ = CloseHandle(token);
            read.ok()?;
            let sid = (*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL)).Label.Sid;
            let count = *GetSidSubAuthorityCount(sid);
            (count > 0).then(|| *GetSidSubAuthority(sid, u32::from(count) - 1))
        }
    }

    // `security`: the open checks of check 1 alone.
    pub fn security_command() -> i32 {
        let mut r = Report {
            pass: true,
            lines: Vec::new(),
        };
        if let Err(e) = impersonation(&mut r) {
            r.check(false, format!("error: {e:#}"));
        }
        finish(&r)
    }

    // `low-integrity-probe <path>`: what a process at this one's integrity
    // gets from the device at `path`. Prints its integrity level, the open's
    // result and, if it opened, GET_INFO's and an ADD's.
    pub fn low_integrity_probe(args: &[String]) -> i32 {
        println!(
            "integrity {}",
            own_integrity().map_or("unknown".to_string(), |rid| format!("{rid:#x}"))
        );
        let Some(path) = args.first() else {
            println!("usage: low-integrity-probe <interface path>");
            return 2;
        };
        let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                0xC000_0000,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )
        };
        let handle = match handle {
            Ok(h) => h,
            Err(e) => {
                println!(
                    "open: error {}",
                    win32_code(&e).unwrap_or(e.code().0 as u32)
                );
                return 0;
            }
        };
        println!("open: ok");
        let mut info = [0u8; 28];
        let mut returned = 0u32;
        let res = unsafe {
            DeviceIoControl(
                handle,
                dd::IOCTL_GET_INFO,
                None,
                0,
                Some(info.as_mut_ptr().cast()),
                info.len() as u32,
                Some(&mut returned),
                None,
            )
        };
        println!("GET_INFO: {}", describe(&res));
        let req = mode_request(1366, 1024, SEED_BASE + 11);
        let mut out = AddMonitorOut::default();
        let res = unsafe {
            DeviceIoControl(
                handle,
                dd::IOCTL_ADD_MONITOR,
                Some(as_bytes(&req).as_ptr().cast()),
                size_of::<AddMonitorIn>() as u32,
                Some((&mut out as *mut AddMonitorOut).cast()),
                size_of::<AddMonitorOut>() as u32,
                Some(&mut returned),
                None,
            )
        };
        println!("ADD: {}", describe(&res));
        unsafe {
            let _ = CloseHandle(handle);
        }
        0
    }

    // ---- closing handles during arrival and swap-chain assignment ----

    // How a churn ADD ended.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Outcome {
        // The ADD succeeded (and the handle close departed the monitor).
        Added,
        // The driver cancelled the ADD because the handle closed meanwhile.
        Cancelled,
        // Another Win32 error.
        Failed(u32),
        // The overlapped request never completed.
        Pending,
    }

    impl Outcome {
        fn from_win32(code: u32) -> Self {
            match code {
                0 => Self::Added,
                errors::OPERATION_ABORTED => Self::Cancelled,
                code => Self::Failed(code),
            }
        }
    }

    const BUSY: Outcome = Outcome::Failed(errors::ALREADY_EXISTS);

    // Retries while the previous churn monitor, with the same seed, is still
    // departing.
    fn until_seed_free(
        mut attempt: impl FnMut() -> anyhow::Result<Outcome>,
    ) -> anyhow::Result<Outcome> {
        for _ in 0..40 {
            let outcome = attempt()?;
            if outcome != BUSY {
                return Ok(outcome);
            }
            sleep(50);
        }
        attempt()
    }

    // ADD on a fresh client handle, wait `delay_ms`, close it. Returns how
    // the ADD went and whether the monitor had a surface (a swap chain) by then.
    fn add_then_close(seed: u32, delay_ms: u64) -> anyhow::Result<(Outcome, bool)> {
        let dev = Driver::open()?;
        let mut added = None;
        let outcome = until_seed_free(|| {
            Ok(match dev.add_monitor(&mode_request(1366, 1024, seed)) {
                Ok(a) => {
                    added = Some(a);
                    Outcome::Added
                }
                Err(e) => Outcome::from_win32(win32_code(&e).unwrap_or(e.code().0 as u32)),
            })
        })?;
        sleep(delay_ms);
        let surface = added
            .and_then(|a| dev.query_monitor(a.monitor_id).ok())
            .is_some_and(|q| q.has(FLAG_SURFACE));
        Ok((outcome, surface))
    }

    // An overlapped ADD on a fresh handle that is closed `delay_ms` after the
    // request goes out, without waiting for it, so the close can overtake it.
    fn add_then_close_overlapped(seed: u32, delay_ms: u64) -> anyhow::Result<Outcome> {
        let path = dd::interface_list()?;
        unsafe {
            let handle = CreateFileW(
                PCWSTR(path.as_ptr()),
                0xC000_0000,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )?;
            let event = CreateEventW(None, true, false, None)?;
            // The driver writes into these after the handle is gone, so they
            // live on the heap until the request completes.
            let mut io = Box::new((
                OVERLAPPED {
                    hEvent: event,
                    ..Default::default()
                },
                mode_request(1366, 1024, seed),
                AddMonitorOut::default(),
            ));
            let (overlapped, request, out) = &mut *io;
            let sent = DeviceIoControl(
                handle,
                dd::IOCTL_ADD_MONITOR,
                Some((request as *const AddMonitorIn).cast()),
                size_of::<AddMonitorIn>() as u32,
                Some((out as *mut AddMonitorOut).cast()),
                size_of::<AddMonitorOut>() as u32,
                None,
                Some(overlapped),
            );
            let early = match &sent {
                Ok(()) => None,
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => None,
                Err(e) => Some(Outcome::from_win32(
                    win32_code(e).unwrap_or(e.code().0 as u32),
                )),
            };
            sleep(delay_ms);
            let _ = CloseHandle(handle);
            if let Some(outcome) = early {
                let _ = CloseHandle(event);
                return Ok(outcome);
            }
            if WaitForSingleObject(event, 10_000) != WAIT_OBJECT_0 {
                // Still owned by the I/O manager: leak it rather than free it.
                Box::leak(io);
                return Ok(Outcome::Pending);
            }
            let _ = CloseHandle(event);
            let status = NTSTATUS(io.0.Internal as i32);
            Ok(Outcome::from_win32(RtlNtStatusToDosError(status)))
        }
    }

    fn tally(outcomes: &BTreeMap<Outcome, usize>) -> String {
        outcomes
            .iter()
            .map(|(o, n)| match o {
                Outcome::Failed(code) => format!("{n} error {code}"),
                _ => format!("{n} {o:?}").to_lowercase(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    // Clients that close their handle right after ADD, as a helper that exits
    // or crashes at once does: synchronous ADDs closed after delays that span
    // arrival and Windows assigning the swap chain, then overlapped ADDs closed
    // before they complete. Once with no other monitor, once next to a monitor
    // of a watch handle. A driver-host crash would break the watch handle (and
    // depart its monitor); a leak would leave monitors no handle owns.
    fn churn(r: &mut Report, spaced: Spaced) -> anyhow::Result<()> {
        const DELAYS: [u64; 13] = [0, 25, 50, 75, 100, 125, 150, 200, 250, 300, 400, 600, 1000];
        const OVERLAPPED_DELAYS: [u64; 12] = [0, 0, 0, 10, 25, 50, 75, 100, 150, 200, 300, 500];
        let (sync_seed, overlapped_seed) = (SEED_BASE + 30, SEED_BASE + 31);

        let since = CrashWatch::new();
        let watch = Driver::open()?;
        let mut kept: Option<u32> = None;
        let alive = |kept: Option<u32>| {
            watch.info().is_ok()
                && kept.is_none_or(|id| watch.query_monitor(id).is_ok_and(|q| q.has(FLAG_PRESENT)))
        };
        for phase in 0..2 {
            let neighbour = if phase == 1 {
                let add = watch.add_monitor(&mode_request(1500, 1000, SEED_BASE + 1))?;
                let q = wait_query(
                    &watch,
                    add.monitor_id,
                    10_000,
                    |_| {},
                    |q| q.has(FLAG_SURFACE),
                )?;
                kept = Some(add.monitor_id);
                format!(
                    "next to the watch handle's monitor {} (surface {})",
                    add.monitor_id,
                    q.has(FLAG_SURFACE)
                )
            } else {
                "with no other monitor".to_string()
            };

            let (mut sync, mut overlapped) = (BTreeMap::new(), BTreeMap::new());
            let (mut closes, mut surfaced, mut survived) = (0, 0, true);
            for delay in DELAYS {
                let (outcome, surface) = add_then_close(sync_seed, delay)?;
                *sync.entry(outcome).or_insert(0) += 1;
                surfaced += usize::from(surface);
                closes += 1;
                survived = alive(kept);
                if !survived {
                    break;
                }
            }
            for delay in OVERLAPPED_DELAYS {
                if !survived {
                    break;
                }
                let outcome =
                    until_seed_free(|| add_then_close_overlapped(overlapped_seed, delay))?;
                *overlapped.entry(outcome).or_insert(0) += 1;
                closes += 1;
                survived = alive(kept);
            }
            sleep(1000);
            survived &= alive(kept);
            r.check(
                survived,
                format!(
                    "{neighbour}: the driver host survived {closes} handles closed right after \
                     ADD (synchronous: {}, {surfaced} with a swap chain at close; overlapped: {})",
                    tally(&sync),
                    tally(&overlapped)
                ),
            );
            let want = usize::from(phase == 1);
            let active = watch.info().ok().map(|i| i.active_monitors as usize);
            let xec = wait_xec(want, 5000);
            r.check(
                active == Some(want) && xec == want,
                format!(
                    "{neighbour}: the closed handles left nothing behind (driver active \
                     {active:?}, DISPLAY\\XEC* {xec}, want {want})"
                ),
            );
            if !survived {
                break;
            }
        }
        drop(watch);
        sleep(500);
        let after = Driver::open()?.info()?.active_monitors;
        let left = wait_xec(0, 8000);
        r.check(
            after == 0 && left == 0,
            format!(
                "closing the watch handle departed its monitor (active {after}, \
                 DISPLAY\\XEC* {left})"
            ),
        );
        no_crash(r, &since, "closes back to back");
        spaced_churn(r, spaced)
    }

    // The spaced phase of check 10: `count` closes, each `min_ms` to `max_ms`
    // after its ADD and followed by SPACED_PAUSE_MS.
    #[derive(Clone, Copy)]
    struct Spaced {
        count: usize,
        min_ms: u64,
        max_ms: u64,
    }

    const SPACED: Spaced = Spaced {
        count: 20,
        min_ms: 100,
        max_ms: 300,
    };
    // A host that died in IddCx's "driver did not release a frame" watchdog
    // did so about two seconds after the close.
    const SPACED_PAUSE_MS: u64 = 3000;

    // Closes spaced out, one every few seconds, as from a helper that exits
    // soon after it starts. A close 100-300 ms after ADD lands while Windows
    // assigns the new monitor its swap chain, and a host that crashed there
    // could do so seconds later, after a back-to-back check had moved on. So
    // each close is followed by a pause and a check of the driver host: a
    // long-lived handle still answers GET_INFO, the same WUDFHost process
    // runs, and no crash signal appeared.
    fn spaced_churn(r: &mut Report, spaced: Spaced) -> anyhow::Result<()> {
        let seed = SEED_BASE + 32;
        let watch = Driver::open()?;
        let first = Hosts::now();
        let (mut sync, mut overlapped) = (BTreeMap::new(), BTreeMap::new());
        let span = spaced.max_ms.saturating_sub(spaced.min_ms) + 1;
        let mut failure = None;
        let mut closes = 0;
        for i in 0..spaced.count {
            // Spread over the range, for both kinds of ADD.
            let delay = spaced.min_ms + (i as u64 * 37) % span;
            let kind = if i % 2 == 0 {
                "synchronous"
            } else {
                "overlapped"
            };
            let since = CrashWatch::new();
            let before = Hosts::now();
            let outcome = if i % 2 == 0 {
                add_then_close(seed, delay).map(|(o, _)| o)
            } else {
                until_seed_free(|| add_then_close_overlapped(seed, delay))
            };
            let outcome = match outcome {
                Ok(o) => o,
                Err(e) => {
                    failure = Some(format!("close {}: {kind} ADD: {e:#}", i + 1));
                    break;
                }
            };
            let outcomes = if i % 2 == 0 {
                &mut sync
            } else {
                &mut overlapped
            };
            *outcomes.entry(outcome).or_insert(0) += 1;
            closes += 1;
            sleep(SPACED_PAUSE_MS);
            let after = Hosts::now();
            let alive = watch.info().is_ok();
            let signals = since.signals();
            if !alive || !before.unchanged(&after) || !signals.is_empty() {
                failure = Some(format!(
                    "close {} ({kind}, {delay} ms after ADD, {outcome:?}): the long-lived \
                     handle {}, {} before, {} after{}",
                    i + 1,
                    if alive { "answers" } else { "is dead" },
                    before.describe(),
                    after.describe(),
                    if signals.is_empty() {
                        String::new()
                    } else {
                        format!("; {}", signals.join("; "))
                    }
                ));
                break;
            }
        }
        let ok = failure.is_none();
        r.check(
            ok,
            format!(
                "{closes} handles closed {}-{} ms after ADD, each followed by a {} s pause \
                 (synchronous: {}; overlapped: {}): {}",
                spaced.min_ms,
                spaced.max_ms,
                SPACED_PAUSE_MS / 1000,
                tally(&sync),
                tally(&overlapped),
                failure.unwrap_or_else(|| format!(
                    "after each the long-lived handle answered, {} ran throughout and no \
                     crash signal appeared",
                    first.describe()
                ))
            ),
        );
        if ok {
            let active = watch.info().ok().map(|i| i.active_monitors);
            let xec = wait_xec(0, 5000);
            r.check(
                active == Some(0) && xec == 0,
                format!(
                    "the spaced closes left nothing behind (driver active {active:?}, \
                     DISPLAY\\XEC* {xec})"
                ),
            );
        }
        Ok(())
    }

    // `churn [<count> [<min-ms> <max-ms>]]`: check 10 (and 12) alone.
    pub fn churn_command(args: &[String]) -> i32 {
        let spaced = spaced_args(args);
        keep_display_on();
        let crash = CrashWatch::new();
        let mut r = Report {
            pass: true,
            lines: Vec::new(),
        };
        let saved = (
            query_config(QDC_ALL_PATHS),
            query_config(QDC_ONLY_ACTIVE_PATHS),
        );
        r.info(format!("driver host at start: {}", Hosts::now().describe()));
        if let Err(e) = churn(&mut r, spaced) {
            r.check(false, format!("error: {e:#}"));
        }
        if let (Ok(all), Ok(active)) = &saved {
            r.info(format!(
                "restored the saved display configuration: {}",
                apply(all, active)
            ));
        }
        r.info(format!("driver host at end: {}", Hosts::now().describe()));
        no_crash(&mut r, &crash, "whole run");
        display_idle();
        finish(&r)
    }

    // `spaced [<count> [<min-ms> <max-ms>]]`: the spaced phase of check 10
    // (and 12) alone.
    pub fn spaced_command(args: &[String]) -> i32 {
        let spaced = spaced_args(args);
        keep_display_on();
        let crash = CrashWatch::new();
        let mut r = Report {
            pass: true,
            lines: Vec::new(),
        };
        let saved = (
            query_config(QDC_ALL_PATHS),
            query_config(QDC_ONLY_ACTIVE_PATHS),
        );
        let start_active = active_set();
        r.info(format!("driver host at start: {}", Hosts::now().describe()));
        if let Err(e) = spaced_churn(&mut r, spaced) {
            r.check(false, format!("error: {e:#}"));
        }
        if let (Ok(all), Ok(active)) = &saved {
            if active_set() != start_active {
                r.info(format!(
                    "restored the saved display configuration: {}",
                    apply(all, active)
                ));
            }
        }
        r.info(format!("driver host at end: {}", Hosts::now().describe()));
        no_crash(&mut r, &crash, "whole run");
        display_idle();
        finish(&r)
    }

    fn spaced_args(args: &[String]) -> Spaced {
        let number =
            |i: usize, default| args.get(i).and_then(|a| a.parse().ok()).unwrap_or(default);
        Spaced {
            count: number(0, SPACED.count as u64) as usize,
            min_ms: number(1, SPACED.min_ms),
            max_ms: number(2, SPACED.max_ms),
        }
    }

    fn finish(r: &Report) -> i32 {
        for l in &r.lines {
            println!("{l}");
        }
        println!("==== {} ====", if r.pass { "PASS" } else { "FAIL" });
        i32::from(!r.pass)
    }

    // ---- the driver host's handles ----

    // The driver host's process id and handle count, when this user may open
    // it (an elevated administrator can; the signed-in user usually cannot).
    fn host_handles() -> Option<(u32, u32)> {
        let pid = Hosts::now().ours?;
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut count = 0u32;
            let read = GetProcessHandleCount(process, &mut count);
            let _ = CloseHandle(process);
            read.ok().map(|()| (pid, count))
        }
    }

    // Monitors created and departed one after another, as a helper that
    // extends again and again does: each becomes a desktop target and gets a
    // swap chain and a frame, then departs, by REMOVE (even cycles) or by
    // closing its handle (odd ones). The driver releases everything it holds
    // per monitor, but Windows itself does not: on build 26200 the driver
    // host keeps about two handles per swap chain (opened by DXGI for
    // IddCx's IddCxSwapChainSetDevice, dxgi!CDXGIIndirectSwapChain::Open, and
    // put there by the compositor's DXGI), so about four per cycle (Windows
    // assigns each new monitor two swap chains), and the NVIDIA user-mode
    // driver one per load; see "Driver host handles" in the driver README.
    // So the handle count, measured after each cycle when readable, may grow
    // after the first two cycles (which warm up thread pools and caches) by
    // at most 8 per cycle; that still catches anything the driver would leak
    // per swap chain (a D3D device is about 80 handles, a shared texture
    // about 20).
    fn cycles(r: &mut Report, count: usize, touched: &mut bool) -> anyhow::Result<()> {
        let seed = SEED_BASE + 41;
        let hosts = Hosts::now();
        let watch = Driver::open()?;
        let before = host_handles();
        let mut counts = Vec::new();
        let (mut surfaced, mut framed) = (0, 0);
        for i in 0..count {
            let dev = Driver::open()?;
            let mut added = None;
            until_seed_free(|| {
                Ok(match dev.add_monitor(&mode_request(1366, 1024, seed)) {
                    Ok(a) => {
                        added = Some(a);
                        Outcome::Added
                    }
                    Err(e) => Outcome::from_win32(win32_code(&e).unwrap_or(e.code().0 as u32)),
                })
            })?;
            let add = added.ok_or_else(|| anyhow::anyhow!("cycle {}: ADD failed", i + 1))?;
            // A swap chain and a frame mean Windows made it a desktop target;
            // if it does not by itself, extend the desktop onto it.
            let shown = |q: &QueryMonitorOut| q.has(FLAG_SURFACE) && q.has(FLAG_FRAME);
            let mut q = wait_query(&dev, add.monitor_id, 4000, |_| {}, shown)?;
            if !shown(&q) {
                extend();
                *touched = true;
                q = wait_query(&dev, add.monitor_id, 8000, |_| {}, shown)?;
            }
            surfaced += usize::from(q.has(FLAG_SURFACE));
            framed += usize::from(q.has(FLAG_FRAME));
            if i % 2 == 0 {
                dev.remove_monitor(add.monitor_id)?;
            }
            drop(dev);
            wait_xec(0, 8000);
            sleep(1500);
            if let Some((_, n)) = host_handles() {
                counts.push(n);
            }
        }
        let now = Hosts::now();
        r.check(
            surfaced == count && framed == count,
            format!(
                "{count} monitors added, given a swap chain ({surfaced}) and a frame \
                 ({framed}), then departed (by REMOVE and by handle close in turn){}",
                if surfaced == count {
                    String::new()
                } else {
                    format!(" ({NO_SURFACE})")
                }
            ),
        );
        r.check(
            watch.info().is_ok_and(|i| i.active_monitors == 0)
                && hosts.unchanged(&now)
                && xec_monitors() == 0,
            format!(
                "after the cycles nothing is left and the {} ran throughout",
                hosts.describe()
            ),
        );
        match before {
            Some((pid, start)) if counts.len() == count && count >= 6 => {
                let base = counts[1];
                let last = *counts.last().unwrap_or(&base);
                let cycles = count as u32 - 2;
                let allowed = 8 * cycles;
                let growth = i64::from(last) - i64::from(base);
                r.check(
                    last <= base + allowed,
                    format!(
                        "driver host (pid {pid}) handle count over {count} cycles: {start} \
                         before, then {counts:?}: {growth} after the warm-up, {:.1} per cycle \
                         (allowed 8: Windows keeps about 4)",
                        growth as f64 / f64::from(cycles)
                    ),
                );
            }
            _ => r.info(format!(
                "driver host handle count not readable here ({}); remote.sh driver-accept \
                 prints it before and after",
                if before.is_some() {
                    "fewer than 6 cycles"
                } else {
                    "no access to the WUDFHost process"
                }
            )),
        }
        Ok(())
    }

    // `cycles [<count>]`: the handle-leak check alone (default 20 cycles).
    pub fn cycles_command(args: &[String]) -> i32 {
        unsafe {
            let _ = SetProcessDpiAwareness(PROCESS_PER_MONITOR_DPI_AWARE);
        }
        let count = args.first().and_then(|a| a.parse().ok()).unwrap_or(20);
        keep_display_on();
        let crash = CrashWatch::new();
        let mut r = Report {
            pass: true,
            lines: Vec::new(),
        };
        let saved = (
            query_config(QDC_ALL_PATHS),
            query_config(QDC_ONLY_ACTIVE_PATHS),
        );
        let start_active = active_set();
        let mut touched = false;
        if let Err(e) = cycles(&mut r, count, &mut touched) {
            r.check(false, format!("error: {e:#}"));
        }
        if let (Ok(all), Ok(active)) = &saved {
            if touched || active_set() != start_active {
                r.info(format!(
                    "restored the saved display configuration: {}",
                    apply(all, active)
                ));
            }
        }
        sleep(500);
        r.check(active_set() == start_active, "display topology restored");
        no_crash(&mut r, &crash, "whole run");
        display_idle();
        finish(&r)
    }

    // `open <luid-low> <luid-high> <name> [abandon]`.
    pub fn open_command(args: &[String]) -> i32 {
        let parsed = (|| -> anyhow::Result<(QueryMonitorOut, bool)> {
            anyhow::ensure!(
                args.len() >= 3,
                "usage: open <luid-low> <luid-high> <name> [abandon]"
            );
            let mut q = QueryMonitorOut {
                flags: FLAG_PRESENT | FLAG_SURFACE,
                render_luid_low: args[0].parse()?,
                render_luid_high: args[1].parse()?,
                ..Default::default()
            };
            let name: Vec<u16> = args[2].encode_utf16().collect();
            anyhow::ensure!(name.len() < dd::TEXTURE_NAME_LEN, "name too long");
            q.texture_name[..name.len()].copy_from_slice(&name);
            Ok((q, args.get(3).map(String::as_str) == Some("abandon")))
        })();
        let (q, abandon) = match parsed {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{e:#}");
                return 2;
            }
        };
        let (device, _context) = match create_device(q.render_luid()) {
            Ok(d) => d,
            Err(e) => {
                println!("create device: {e}");
                return 3;
            }
        };
        let surface = match SharedSurface::open(&device, &q) {
            Ok(s) => s,
            Err(e) => {
                println!(
                    "open {}: {:#010x} {}",
                    q.texture_name(),
                    e.code().0,
                    e.message()
                );
                return 4;
            }
        };
        println!("opened {}", q.texture_name());
        if !abandon {
            return 0;
        }
        match surface.acquire(3000) {
            // Exit holding the frame: process teardown abandons the keyed mutex.
            Acquire::Frame => std::process::exit(0),
            other => {
                println!("acquire: {other:?}");
                5
            }
        }
    }

    // `sustain [<seconds>]`: one monitor in ordinary use.
    pub fn sustain_command(args: &[String]) -> i32 {
        unsafe {
            let _ = SetProcessDpiAwareness(PROCESS_PER_MONITOR_DPI_AWARE);
        }
        let secs = args.first().and_then(|a| a.parse().ok()).unwrap_or(30);
        keep_display_on();
        let crash = CrashWatch::new();
        let mut r = Report {
            pass: true,
            lines: Vec::new(),
        };
        let saved = (
            query_config(QDC_ALL_PATHS),
            query_config(QDC_ONLY_ACTIVE_PATHS),
        );
        let start_active = active_set();
        let mut touched = false;
        if let Err(e) = sustain(&mut r, secs, &mut touched) {
            r.check(false, format!("error: {e:#}"));
        }
        if let (Ok(all), Ok(active)) = &saved {
            if touched || active_set() != start_active {
                r.info(format!(
                    "restored the saved display configuration: {}",
                    apply(all, active)
                ));
            }
        }
        sleep(500);
        r.check(active_set() == start_active, "display topology restored");
        no_crash(&mut r, &crash, "whole run");
        display_idle();
        finish(&r)
    }

    // ADD, stream for `secs`, REMOVE; then ADD, stream for 10 s and close the
    // handle; with a long-lived handle and the WUDFHost process checked at
    // the end.
    fn sustain(r: &mut Report, secs: u64, touched: &mut bool) -> anyhow::Result<()> {
        let seed = SEED_BASE + 40;
        let watch = Driver::open()?;
        let hosts = Hosts::now();
        r.info(format!("driver host at start: {}", hosts.describe()));

        let dev = Driver::open()?;
        let add = dev.add_monitor(&mode_request(1366, 1024, seed))?;
        let (frames, seen, painted) = stream(&dev, add, secs, touched)?;
        r.check(
            frames > 0 && seen == painted,
            format!(
                "monitor {} streamed for {secs} s: {frames} frames read on S_OK, \
                 {seen}/{painted} painted colours seen",
                add.monitor_id
            ),
        );
        dev.remove_monitor(add.monitor_id)?;
        let q = dev.query_monitor(add.monitor_id);
        let active = dev.info()?.active_monitors;
        let xec = wait_xec(0, 8000);
        r.check(
            err_code(&q) == Some(errors::NOT_FOUND) && active == 0 && xec == 0,
            format!(
                "REMOVE after streaming departed it (QUERY {}, active {active}, \
                 DISPLAY\\XEC* {xec})",
                describe(&q)
            ),
        );
        drop(dev);

        let dev = Driver::open()?;
        let add = dev.add_monitor(&mode_request(1366, 1024, seed))?;
        let (frames, seen, painted) = stream(&dev, add, 10, touched)?;
        drop(dev);
        sleep(500);
        let active = watch.info()?.active_monitors;
        let xec = wait_xec(0, 8000);
        r.check(
            frames > 0 && seen == painted && active == 0 && xec == 0,
            format!(
                "monitor {} streamed for 10 s ({frames} frames, {seen}/{painted} colours), \
                 then closing its handle departed it (active {active}, DISPLAY\\XEC* {xec})",
                add.monitor_id
            ),
        );

        let now = Hosts::now();
        r.check(
            watch.info().is_ok() && hosts.unchanged(&now),
            format!(
                "the long-lived handle answers and the {} ran throughout (now {})",
                hosts.describe(),
                now.describe()
            ),
        );
        Ok(())
    }

    // Makes the monitor a desktop target, then for `secs` paints colours on it
    // and reads frames through the keyed mutex, reopening the texture when the
    // driver replaces it. Returns the frames read on S_OK, the colours seen
    // and the colours painted.
    fn stream(
        dev: &Driver,
        add: AddMonitorOut,
        secs: u64,
        touched: &mut bool,
    ) -> anyhow::Result<(usize, usize, usize)> {
        let id = add.monitor_id;
        let target = (add.os_adapter_luid(), add.os_target_id);
        if !wait_active(&[target], 4000) {
            extend();
            *touched = true;
        }
        anyhow::ensure!(
            wait_active(&[target], 8000),
            "monitor {id} did not become a desktop target"
        );
        let q = wait_query(dev, id, 10_000, |_| {}, |q| q.has(FLAG_SURFACE))?;
        anyhow::ensure!(
            q.has(FLAG_SURFACE),
            "monitor {id} has no surface: {NO_SURFACE}"
        );
        let rect = target_rect(target)
            .ok_or_else(|| anyhow::anyhow!("monitor {id} has no desktop rect"))?;
        let painter = Painter::new(rect)?;
        let mut reader = Reader::new(&q)?;
        // A new window takes a moment to reach the monitor's frames: wait for
        // a first colour before counting.
        painter.paint(0x808080);
        reader.wait_colour(0x808080, 3000)?;
        let deadline = Instant::now() + Duration::from_secs(secs);
        let (mut frames, mut seen, mut painted) = (0, 0, 0);
        while Instant::now() < deadline {
            let rgb = colour(painted);
            painter.paint(rgb);
            painted += 1;
            let until = Instant::now() + Duration::from_millis(1000);
            while Instant::now() < until {
                if reader.surface.is_none() {
                    // Withdrawn for a reassign; wait for the next generation.
                    sleep(50);
                    reader.refresh(&dev.query_monitor(id)?)?;
                    continue;
                }
                match reader.read(50)? {
                    (Acquire::Frame, px) => {
                        frames += 1;
                        if px == Some(rgb) {
                            seen += 1;
                            break;
                        }
                    }
                    (Acquire::Timeout, _) => {}
                    (_, _) => {
                        reader.refresh(&dev.query_monitor(id)?)?;
                    }
                }
            }
            sleep(30);
        }
        Ok((frames, seen, painted))
    }

    // `state`: what is there now.
    pub fn state_command() -> i32 {
        match Driver::open().and_then(|d| d.info()) {
            Ok(i) => println!(
                "GET_INFO: protocol v{} driver {} active {}",
                i.protocol_version,
                i.driver_version(),
                i.active_monitors
            ),
            Err(e) => println!("GET_INFO: {e}"),
        }
        println!("DISPLAY\\XEC* monitors: {}", xec_monitors());
        let paths = active_set();
        println!("active display paths: {}", paths.len());
        for (source_adapter, source, target_adapter, target) in paths {
            println!("  source {source_adapter:x}/{source} -> target {target_adapter:x}/{target}");
        }
        println!("{}", Hosts::now().describe());
        0
    }

    // `monitor <width> <height> <seconds>`.
    pub fn monitor_command(args: &[String]) -> i32 {
        let run = || -> anyhow::Result<()> {
            anyhow::ensure!(args.len() == 3, "usage: monitor <width> <height> <seconds>");
            let (w, h, secs): (u32, u32, u64) =
                (args[0].parse()?, args[1].parse()?, args[2].parse()?);
            let saved = (
                query_config(QDC_ALL_PATHS)?,
                query_config(QDC_ONLY_ACTIVE_PATHS)?,
            );
            let dev = Driver::open()?;
            let add = dev.add_monitor(&mode_request(w, h, SEED_BASE + 10))?;
            let target = (add.os_adapter_luid(), add.os_target_id);
            let extended = !wait_active(&[target], 4000);
            if extended {
                extend();
            }
            let q = wait_query(
                &dev,
                add.monitor_id,
                10_000,
                |_| {},
                |q| q.has(FLAG_SURFACE),
            )?;
            println!(
                "monitor {} {}x{} luid {} {} name {}",
                add.monitor_id,
                q.width,
                q.height,
                q.render_luid_low,
                q.render_luid_high,
                q.texture_name()
            );
            sleep(secs * 1000);
            drop(dev);
            sleep(500);
            if extended {
                apply(&saved.0, &saved.1);
            }
            Ok(())
        };
        match run() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("{e:#}");
                1
            }
        }
    }
}
