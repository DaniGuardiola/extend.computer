//! Client for the extend.computer indirect display driver
//! (`native/windows/driver`; the contract is its `Protocol.h` and README). It
//! opens the driver's device interface, creates and removes virtual monitors,
//! and reads their frames from each monitor's keyed-mutex shared texture.
//! Shared by the driver acceptance tool and the display helper; include it with
//! `#[path = "../../shared/display_driver.rs"] mod display_driver;`.
//!
//! It needs these `windows` crate features: `Win32_Foundation`,
//! `Win32_Devices_DeviceAndDriverInstallation`, `Win32_Security`,
//! `Win32_Storage_FileSystem`, `Win32_System_IO`, `Win32_Graphics_Direct3D`,
//! `Win32_Graphics_Direct3D11`, `Win32_Graphics_Dxgi` and
//! `Win32_Graphics_Dxgi_Common`.
//!
//! Frame loop: open a [`Driver`], [`Driver::add_monitor`], poll
//! [`Driver::query_monitor`] until [`FLAG_SURFACE`], then [`SharedSurface::open`]
//! on a device for its render adapter ([`create_device`]). Each
//! [`SharedSurface::acquire`] that returns [`Acquire::Frame`] owns the newest
//! frame until [`SharedSurface::release`]. On [`Acquire::Abandoned`], an error,
//! or a stretch of timeouts, query again and [`SharedSurface::reopen_if_stale`]:
//! the driver replaces the texture (a new generation and name) on swap-chain
//! reassigns, mode changes and abandoned keyed mutexes. Dropping the [`Driver`]
//! departs every monitor it created.
#![allow(dead_code)]

use std::mem::size_of;

use windows::core::{Error, Interface, Result, GUID, HRESULT, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW,
    CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CR_BUFFER_SMALL, CR_SUCCESS,
};
use windows::Win32::Foundation::{CloseHandle, ERROR_NOT_FOUND, HANDLE, HMODULE, LUID};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11Device1, ID3D11DeviceContext, ID3D11Texture2D,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory4, IDXGIKeyedMutex,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, SECURITY_IDENTIFICATION,
    SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::IO::DeviceIoControl;

/// Device-interface class `{E053DB0E-1431-463B-ABDC-0CB768D6E649}`.
pub const INTERFACE_GUID: GUID = GUID::from_u128(0xE053DB0E_1431_463B_ABDC_0CB768D6E649);

pub const PROTOCOL_VERSION: u32 = 2;
pub const MAX_MONITORS: u32 = 4;
pub const MAX_MODES: usize = 8;
pub const MIN_DIMENSION: u32 = 320;
pub const MAX_DIMENSION: u32 = 8192;
pub const MAX_REFRESH_HZ: u32 = 1000;
pub const TEXTURE_NAME_LEN: usize = 64;

const fn ctl_code(function: u32) -> u32 {
    // FILE_DEVICE_UNKNOWN, FILE_READ_DATA | FILE_WRITE_DATA, METHOD_BUFFERED.
    (0x22 << 16) | (3 << 14) | (function << 2)
}
pub const IOCTL_GET_INFO: u32 = ctl_code(0x800);
pub const IOCTL_ADD_MONITOR: u32 = ctl_code(0x801);
pub const IOCTL_REMOVE_MONITOR: u32 = ctl_code(0x802);
pub const IOCTL_QUERY_MONITOR: u32 = ctl_code(0x803);
const _: () = assert!(IOCTL_GET_INFO == 0x0022_E000 && IOCTL_QUERY_MONITOR == 0x0022_E00C);

/// [`QueryMonitorOut::flags`]: the monitor exists and has arrived.
pub const FLAG_PRESENT: u32 = 0x1;
/// The shared texture is published; name, LUID, size and format are valid.
pub const FLAG_SURFACE: u32 = 0x2;
/// At least one frame has been published.
pub const FLAG_FRAME: u32 = 0x4;

/// The driver's Win32 errors (`GetLastError` values of failed IOCTLs).
pub mod errors {
    pub const INVALID_FUNCTION: u32 = 1;
    pub const ACCESS_DENIED: u32 = 5;
    pub const INVALID_PARAMETER: u32 = 87;
    pub const INSUFFICIENT_BUFFER: u32 = 122;
    pub const ALREADY_EXISTS: u32 = 183;
    pub const OPERATION_ABORTED: u32 = 995;
    pub const NOT_FOUND: u32 = 1168;
    pub const REVISION_MISMATCH: u32 = 1306;
    pub const BAD_IMPERSONATION_LEVEL: u32 = 1346;
    pub const NO_SYSTEM_RESOURCES: u32 = 1450;
}

/// The Win32 error code inside an `HRESULT_FROM_WIN32` error, if it is one.
pub fn win32_code(error: &Error) -> Option<u32> {
    let hr = error.code().0 as u32;
    (hr & 0xFFFF_0000 == 0x8007_0000).then_some(hr & 0xFFFF)
}

/// `EXTEND_INFO`.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct Info {
    pub struct_size: u32,
    pub protocol_version: u32,
    pub driver_version_high: u32,
    pub driver_version_low: u32,
    pub iddcx_version: u32,
    pub max_monitors: u32,
    pub active_monitors: u32,
}

impl Info {
    /// The driver package's DriverVer, `major.minor.build.revision`.
    pub fn driver_version(&self) -> String {
        let (h, l) = (self.driver_version_high, self.driver_version_low);
        format!("{}.{}.{}.{}", h >> 16, h & 0xFFFF, l >> 16, l & 0xFFFF)
    }
}

/// `EXTEND_MODE`.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
}

impl Mode {
    pub const fn new(width: u32, height: u32, refresh_hz: u32) -> Self {
        Self {
            width,
            height,
            refresh_hz,
        }
    }
}

/// `EXTEND_ADD_MONITOR_IN`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AddMonitorIn {
    pub struct_size: u32,
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub physical_width_mm: u32,
    pub physical_height_mm: u32,
    /// Stable per client device: the EDID serial, so Windows remembers the
    /// monitor's arrangement and scale. Unique among active monitors.
    pub serial_seed: u32,
    pub fallback_mode_count: u32,
    pub fallback_modes: [Mode; MAX_MODES],
}

impl AddMonitorIn {
    /// A request for `mode` with no fallbacks and the default (~96 DPI) size.
    pub fn new(mode: Mode, serial_seed: u32) -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            version: PROTOCOL_VERSION,
            width: mode.width,
            height: mode.height,
            refresh_hz: mode.refresh_hz,
            physical_width_mm: 0,
            physical_height_mm: 0,
            serial_seed,
            fallback_mode_count: 0,
            fallback_modes: [Mode::default(); MAX_MODES],
        }
    }

    /// Adds up to [`MAX_MODES`] fallback modes; extra ones are ignored.
    pub fn with_fallbacks(mut self, modes: &[Mode]) -> Self {
        let n = modes.len().min(MAX_MODES);
        self.fallback_modes[..n].copy_from_slice(&modes[..n]);
        self.fallback_mode_count = n as u32;
        self
    }

    pub fn with_physical_size_mm(mut self, width: u32, height: u32) -> Self {
        self.physical_width_mm = width;
        self.physical_height_mm = height;
        self
    }
}

/// `EXTEND_ADD_MONITOR_OUT`.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct AddMonitorOut {
    pub struct_size: u32,
    pub monitor_id: u32,
    pub os_adapter_luid_low: u32,
    pub os_adapter_luid_high: i32,
    /// The monitor's target id in `QueryDisplayConfig`, on `os_adapter_luid()`.
    pub os_target_id: u32,
    pub connector_index: u32,
}

impl AddMonitorOut {
    pub fn os_adapter_luid(&self) -> LUID {
        LUID {
            LowPart: self.os_adapter_luid_low,
            HighPart: self.os_adapter_luid_high,
        }
    }
}

/// `EXTEND_REMOVE_MONITOR_IN` and `EXTEND_QUERY_MONITOR_IN`, which share a layout.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct MonitorIdIn {
    pub struct_size: u32,
    pub monitor_id: u32,
}

impl MonitorIdIn {
    pub fn new(monitor_id: u32) -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            monitor_id,
        }
    }
}

/// `EXTEND_QUERY_MONITOR_OUT`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct QueryMonitorOut {
    pub struct_size: u32,
    pub monitor_id: u32,
    pub flags: u32,
    /// Bumps whenever a new shared texture replaces the old one; 0 until the first.
    pub surface_generation: u32,
    pub render_luid_low: u32,
    pub render_luid_high: i32,
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub frame_counter_low: u32,
    pub frame_counter_high: u32,
    pub texture_name: [u16; TEXTURE_NAME_LEN],
}

impl Default for QueryMonitorOut {
    fn default() -> Self {
        // SAFETY: plain integers and a u16 array, for which all-zero is valid.
        unsafe { std::mem::zeroed() }
    }
}

impl QueryMonitorOut {
    pub fn has(&self, flag: u32) -> bool {
        self.flags & flag != 0
    }

    /// Frames published so far, monotonic across generations.
    pub fn frame_counter(&self) -> u64 {
        (u64::from(self.frame_counter_high) << 32) | u64::from(self.frame_counter_low)
    }

    pub fn render_luid(&self) -> LUID {
        LUID {
            LowPart: self.render_luid_low,
            HighPart: self.render_luid_high,
        }
    }

    /// The texture name, NUL-terminated whatever the driver sent.
    pub fn texture_name_z(&self) -> [u16; TEXTURE_NAME_LEN] {
        let mut name = self.texture_name;
        name[TEXTURE_NAME_LEN - 1] = 0;
        name
    }

    pub fn texture_name(&self) -> String {
        let end = self
            .texture_name
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(TEXTURE_NAME_LEN);
        String::from_utf16_lossy(&self.texture_name[..end])
    }
}

/// A protocol struct: `#[repr(C)]`, only integer fields, no padding, every bit
/// pattern valid, so it can be sent and received as raw bytes.
///
/// # Safety
///
/// Implement only for types that meet all of the above.
pub unsafe trait Plain: Copy {}

// SAFETY: each is #[repr(C)] with only u32/i32/u16 fields and, as the size
// asserts below show, no padding.
unsafe impl Plain for Info {}
unsafe impl Plain for Mode {}
unsafe impl Plain for AddMonitorIn {}
unsafe impl Plain for AddMonitorOut {}
unsafe impl Plain for MonitorIdIn {}
unsafe impl Plain for QueryMonitorOut {}

const _: () = assert!(size_of::<Info>() == 28);
const _: () = assert!(size_of::<Mode>() == 12);
const _: () = assert!(size_of::<AddMonitorIn>() == 132);
const _: () = assert!(size_of::<AddMonitorOut>() == 24);
const _: () = assert!(size_of::<MonitorIdIn>() == 8);
const _: () = assert!(size_of::<QueryMonitorOut>() == 172);

/// An open handle to the driver's control interface. Monitors belong to the
/// handle that created them; dropping it departs them.
pub struct Driver {
    handle: HANDLE,
}

// SAFETY: a device handle may be used and closed from any thread.
unsafe impl Send for Driver {}
unsafe impl Sync for Driver {}

impl Driver {
    /// Opens the first present driver interface. The handle carries
    /// identification-level impersonation, which ADD_MONITOR needs to scope the
    /// monitor's texture to this logon session.
    pub fn open() -> Result<Self> {
        let list = interface_list()?;
        unsafe {
            // The list's first entry is NUL-terminated in place.
            let handle = CreateFileW(
                PCWSTR(list.as_ptr()),
                0xC000_0000, // GENERIC_READ | GENERIC_WRITE
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )?;
            Ok(Self { handle })
        }
    }

    pub fn info(&self) -> Result<Info> {
        let mut out = Info::default();
        self.ioctl(IOCTL_GET_INFO, &[], as_bytes_mut(&mut out))?;
        Ok(out)
    }

    pub fn add_monitor(&self, request: &AddMonitorIn) -> Result<AddMonitorOut> {
        let mut out = AddMonitorOut::default();
        self.ioctl(IOCTL_ADD_MONITOR, as_bytes(request), as_bytes_mut(&mut out))?;
        Ok(out)
    }

    pub fn remove_monitor(&self, monitor_id: u32) -> Result<()> {
        let input = MonitorIdIn::new(monitor_id);
        self.ioctl(IOCTL_REMOVE_MONITOR, as_bytes(&input), &mut [])?;
        Ok(())
    }

    pub fn query_monitor(&self, monitor_id: u32) -> Result<QueryMonitorOut> {
        let input = MonitorIdIn::new(monitor_id);
        let mut out = QueryMonitorOut::default();
        self.ioctl(
            IOCTL_QUERY_MONITOR,
            as_bytes(&input),
            as_bytes_mut(&mut out),
        )?;
        Ok(out)
    }

    /// Sends a raw IOCTL; returns the number of output bytes written.
    pub fn ioctl(&self, code: u32, input: &[u8], output: &mut [u8]) -> Result<u32> {
        let mut returned = 0u32;
        unsafe {
            DeviceIoControl(
                self.handle,
                code,
                (!input.is_empty()).then_some(input.as_ptr().cast()),
                input.len() as u32,
                (!output.is_empty()).then_some(output.as_mut_ptr().cast()),
                output.len() as u32,
                Some(&mut returned),
                None,
            )?;
        }
        Ok(returned)
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

fn not_found(message: &str) -> Error {
    Error::new(HRESULT::from_win32(ERROR_NOT_FOUND.0), message)
}

/// The present driver interfaces, a NUL-separated list ending in two NULs;
/// its first entry is a NUL-terminated path for `CreateFileW`. Retries when an
/// interface arrives between sizing the list and reading it.
pub fn interface_list() -> Result<Vec<u16>> {
    loop {
        unsafe {
            let mut len = 0u32;
            let cr = CM_Get_Device_Interface_List_SizeW(
                &mut len,
                &INTERFACE_GUID,
                PCWSTR::null(),
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            );
            if cr != CR_SUCCESS || len < 2 {
                return Err(not_found("no extend.computer display driver interface"));
            }
            let mut list = vec![0u16; len as usize];
            let cr = CM_Get_Device_Interface_ListW(
                &INTERFACE_GUID,
                PCWSTR::null(),
                &mut list,
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            );
            if cr == CR_BUFFER_SMALL {
                continue;
            }
            if cr != CR_SUCCESS || list[0] == 0 {
                return Err(not_found("no extend.computer display driver interface"));
            }
            return Ok(list);
        }
    }
}

/// The bytes of a protocol struct.
pub fn as_bytes<T: Plain>(value: &T) -> &[u8] {
    // SAFETY: Plain types have no padding, so every byte is initialized.
    unsafe { std::slice::from_raw_parts((value as *const T).cast(), size_of::<T>()) }
}

fn as_bytes_mut<T: Plain>(value: &mut T) -> &mut [u8] {
    // SAFETY: as above, and every bit pattern is a valid Plain value.
    unsafe { std::slice::from_raw_parts_mut((value as *mut T).cast(), size_of::<T>()) }
}

/// A D3D11 device on the adapter a monitor's texture lives on
/// ([`QueryMonitorOut::render_luid`]).
pub fn create_device(luid: LUID) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    unsafe {
        let factory: IDXGIFactory4 = CreateDXGIFactory1()?;
        let adapter: IDXGIAdapter = factory.EnumAdapterByLuid(luid)?;
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
        match (device, context) {
            (Some(device), Some(context)) => Ok((device, context)),
            _ => Err(Error::empty()),
        }
    }
}

/// The result of [`SharedSurface::acquire`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acquire {
    /// `S_OK`: the caller owns the newest frame until [`SharedSurface::release`].
    Frame,
    /// `WAIT_TIMEOUT` (0x102): no new frame within the timeout. IddCx presents
    /// only on change, so an idle desktop times out too. A replaced texture
    /// usually reports `Abandoned`, but one the client was reading at that
    /// moment just times out from then on, so query the generation now and then.
    Timeout,
    /// `WAIT_ABANDONED` (0x80): the texture is dead, because its device died
    /// holding the frame or the driver withdrew it. Query and reopen.
    Abandoned,
    /// Any other result, such as a removed device.
    Failed(HRESULT),
}

// Keyed-mutex keys: the driver writes under key 0 and releases key 1; the
// client reads under key 1 and releases key 0.
const KEY_FREE: u64 = 0;
const KEY_FRAME: u64 = 1;
// DXGI_SHARED_RESOURCE_READ | DXGI_SHARED_RESOURCE_WRITE: exactly what the
// texture's DACL grants the client.
const SHARED_READ_WRITE: u32 = 0x8000_0001;

/// One generation of a monitor's shared texture, opened by name.
pub struct SharedSurface {
    texture: ID3D11Texture2D,
    mutex: IDXGIKeyedMutex,
    generation: u32,
    /// Unique per monitor, generation and adapter start, unlike the
    /// generation, which starts at 1 for every monitor.
    name: [u16; TEXTURE_NAME_LEN],
    desc: D3D11_TEXTURE2D_DESC,
}

impl SharedSurface {
    /// Opens the texture `query` describes on `device`, which must be on the
    /// render adapter.
    pub fn open(device: &ID3D11Device, query: &QueryMonitorOut) -> Result<Self> {
        if !query.has(FLAG_SURFACE) {
            return Err(not_found("the monitor has no shared surface yet"));
        }
        let name = query.texture_name_z();
        unsafe {
            let device1: ID3D11Device1 = device.cast()?;
            let texture: ID3D11Texture2D =
                device1.OpenSharedResourceByName(PCWSTR(name.as_ptr()), SHARED_READ_WRITE)?;
            let mutex: IDXGIKeyedMutex = texture.cast()?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            Ok(Self {
                texture,
                mutex,
                generation: query.surface_generation,
                name,
                desc,
            })
        }
    }

    /// Whether this is still the texture the driver publishes for the monitor
    /// `query` describes. The name tells monitors apart too, so a surface kept
    /// across a REMOVE and a new ADD does not pass for the new monitor's.
    pub fn is_current(&self, query: &QueryMonitorOut) -> bool {
        query.has(FLAG_SURFACE)
            && query.surface_generation == self.generation
            && query.texture_name_z() == self.name
    }

    /// Replaces `surface` when the driver published a new generation; returns
    /// whether it (re)opened one. Clears it while no surface is published.
    pub fn reopen_if_stale(
        surface: &mut Option<SharedSurface>,
        device: &ID3D11Device,
        query: &QueryMonitorOut,
    ) -> Result<bool> {
        if surface.as_ref().is_some_and(|s| s.is_current(query)) {
            return Ok(false);
        }
        *surface = None;
        if !query.has(FLAG_SURFACE) {
            return Ok(false);
        }
        *surface = Some(Self::open(device, query)?);
        Ok(true)
    }

    /// Waits up to `timeout_ms` for a frame. The `windows` crate's
    /// `AcquireSync` folds the success codes `WAIT_TIMEOUT` and
    /// `WAIT_ABANDONED` into `Ok`, so this calls the vtable for the raw HRESULT.
    pub fn acquire(&self, timeout_ms: u32) -> Acquire {
        let hr = unsafe {
            (Interface::vtable(&self.mutex).AcquireSync)(
                Interface::as_raw(&self.mutex),
                KEY_FRAME,
                timeout_ms,
            )
        };
        match hr.0 as u32 {
            0 => Acquire::Frame,
            0x102 => Acquire::Timeout,
            0x80 => Acquire::Abandoned,
            _ => Acquire::Failed(hr),
        }
    }

    /// Hands the frame back to the driver. Call once after each
    /// [`Acquire::Frame`], when done reading (copies queued before this are
    /// ordered before the driver's next write).
    pub fn release(&self) -> Result<()> {
        unsafe { self.mutex.ReleaseSync(KEY_FREE) }
    }

    pub fn texture(&self) -> &ID3D11Texture2D {
        &self.texture
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    pub fn desc(&self) -> &D3D11_TEXTURE2D_DESC {
        &self.desc
    }
}
