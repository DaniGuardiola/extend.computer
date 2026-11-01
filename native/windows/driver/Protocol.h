/*++

    extend.computer Windows indirect-display driver: client control protocol.

    This header is the single source of truth for the IOCTL contract between the
    driver (WUDFHost, LOCAL SERVICE) and a client in the signed-in user's session
    (the display helper or the acceptance tool). The Rust client in
    native/windows/shared/display_driver.rs mirrors these definitions and asserts
    their sizes; keep the two in sync.

    All structures are fixed size with only 32-bit integer fields and 16-bit
    WCHAR arrays, so their layout is identical on every x64/ARM64 Windows ABI with
    no padding. Multi-byte fields are little-endian (native on Windows). Every
    request validates its exact input length and its minimum output length
    before use, and the driver writes exactly sizeof(output struct) bytes.

    Errors are NTSTATUS codes, which a Win32 client sees through GetLastError:
      STATUS_INVALID_PARAMETER (87)      malformed request or out-of-range field
      STATUS_BUFFER_TOO_SMALL (122)      output buffer too small
      STATUS_REVISION_MISMATCH (1306)    EXTEND_ADD_MONITOR_IN.Version is not ours
      STATUS_INSUFFICIENT_RESOURCES (1450) every monitor slot is in use
      STATUS_OBJECT_NAME_COLLISION (183) SerialSeed already used by an active monitor
      STATUS_NOT_FOUND (1168)            no such monitor
      STATUS_ACCESS_DENIED (5)           monitor belongs to another file handle;
                                         or, from CreateFile, the caller is
                                         below Medium integrity or opened with
                                         anonymous QoS (SECURITY_ANONYMOUS)
      STATUS_BAD_IMPERSONATION_LEVEL (1346) ADD could not read the caller's
                                         token (fail closed)
      STATUS_CANCELLED (995)             the handle closed while ADD was in flight
      STATUS_INVALID_DEVICE_REQUEST (1)  unknown IOCTL

    This file is original extend.computer code (not derived from the MS-PL
    IddSampleDriver sample) and carries no Microsoft copyright.

--*/

#pragma once

#ifdef __cplusplus
extern "C" {
#endif

#ifndef CTL_CODE
#include <winioctl.h>
#endif

// Device-interface class GUID the client passes to CM_Get_Device_Interface_ListW.
// {E053DB0E-1431-463B-ABDC-0CB768D6E649}
// DEFINE_GUID is declared by the includer; we expose the raw fields for Rust too.
#define EXTEND_DISPLAY_INTERFACE_GUID_STR L"{E053DB0E-1431-463B-ABDC-0CB768D6E649}"

#define EXTEND_PROTOCOL_VERSION 2u
#define EXTEND_MAX_MONITORS     4u
#define EXTEND_MAX_MODES        8u
#define EXTEND_MIN_DIMENSION    320u  // per-mode width/height range, pixels
#define EXTEND_MAX_DIMENSION    8192u
#define EXTEND_MAX_REFRESH_HZ   1000u // refresh range is 1..EXTEND_MAX_REFRESH_HZ
#define EXTEND_TEXTURE_NAME_LEN 64u   // WCHAR count, NUL-terminated

// IOCTLs: FILE_DEVICE_UNKNOWN (0x22), METHOD_BUFFERED, functions 0x800+,
// access FILE_READ_DATA|FILE_WRITE_DATA. Resulting codes are fixed below.
#define IOCTL_EXTEND_GET_INFO \
    CTL_CODE(FILE_DEVICE_UNKNOWN, 0x800, METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA) // 0x0022E000
#define IOCTL_EXTEND_ADD_MONITOR \
    CTL_CODE(FILE_DEVICE_UNKNOWN, 0x801, METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA) // 0x0022E004
#define IOCTL_EXTEND_REMOVE_MONITOR \
    CTL_CODE(FILE_DEVICE_UNKNOWN, 0x802, METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA) // 0x0022E008
#define IOCTL_EXTEND_QUERY_MONITOR \
    CTL_CODE(FILE_DEVICE_UNKNOWN, 0x803, METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA) // 0x0022E00C

#include <pshpack4.h>

// IOCTL_EXTEND_GET_INFO: no input (length 0), returns this.
typedef struct _EXTEND_INFO {
    UINT32 StructSize;         // = sizeof(EXTEND_INFO) = 28
    UINT32 ProtocolVersion;    // = EXTEND_PROTOCOL_VERSION
    UINT32 DriverVersionHigh;  // DriverVer major << 16 | minor
    UINT32 DriverVersionLow;   // DriverVer build << 16 | revision
    UINT32 IddCxVersion;       // IddCxGetVersion() result, 0 if the call is unavailable
    UINT32 MaxMonitors;        // = EXTEND_MAX_MONITORS
    UINT32 ActiveMonitors;     // monitors currently created across all clients
} EXTEND_INFO;

typedef struct _EXTEND_MODE {
    UINT32 Width;     // EXTEND_MIN_DIMENSION..EXTEND_MAX_DIMENSION
    UINT32 Height;    // EXTEND_MIN_DIMENSION..EXTEND_MAX_DIMENSION
    UINT32 RefreshHz; // 1..EXTEND_MAX_REFRESH_HZ
} EXTEND_MODE;

// IOCTL_EXTEND_ADD_MONITOR input. The driver reads the caller's logon SID at
// identification level and grants only that SID access to the monitor's
// texture. Open the handle with SECURITY_SQOS_PRESENT|SECURITY_IDENTIFICATION,
// which allows exactly that (a handle without SQOS flags allows it too).
typedef struct _EXTEND_ADD_MONITOR_IN {
    UINT32 StructSize;       // = sizeof(EXTEND_ADD_MONITOR_IN) = 132
    UINT32 Version;          // = EXTEND_PROTOCOL_VERSION
    UINT32 Width;            // preferred mode; same ranges as EXTEND_MODE
    UINT32 Height;
    UINT32 RefreshHz;
    UINT32 PhysicalWidthMm;  // EDID physical size; 0 (either) picks ~96 DPI
    UINT32 PhysicalHeightMm;
    UINT32 SerialSeed;       // stable per client device: becomes the EDID serial and
                             // container id, so Windows remembers the monitor's
                             // position and scale; unique among active monitors
    UINT32 FallbackModeCount; // <= EXTEND_MAX_MODES additional modes, all valid
    EXTEND_MODE FallbackModes[EXTEND_MAX_MODES];
} EXTEND_ADD_MONITOR_IN;

// IOCTL_EXTEND_ADD_MONITOR output. The render-adapter LUID and shared-texture
// name are not known until Windows assigns a swap chain, so the client reads
// them from IOCTL_EXTEND_QUERY_MONITOR after the monitor goes active.
typedef struct _EXTEND_ADD_MONITOR_OUT {
    UINT32 StructSize;     // = sizeof(EXTEND_ADD_MONITOR_OUT) = 24
    UINT32 MonitorId;      // opaque, never reused for the life of the adapter
    UINT32 OsAdapterLuidLow;  // IDARG_OUT_MONITORARRIVAL.OsAdapterLuid.LowPart
    INT32  OsAdapterLuidHigh; // IDARG_OUT_MONITORARRIVAL.OsAdapterLuid.HighPart
    UINT32 OsTargetId;     // IDARG_OUT_MONITORARRIVAL.OsTargetId (QueryDisplayConfig target)
    UINT32 ConnectorIndex; // IddCx connector index (device-local slot)
} EXTEND_ADD_MONITOR_OUT;

// IOCTL_EXTEND_REMOVE_MONITOR input. Only the creating handle may remove.
typedef struct _EXTEND_REMOVE_MONITOR_IN {
    UINT32 StructSize; // = sizeof(EXTEND_REMOVE_MONITOR_IN) = 8
    UINT32 MonitorId;
} EXTEND_REMOVE_MONITOR_IN;

// IOCTL_EXTEND_QUERY_MONITOR input. Only the creating handle may query.
typedef struct _EXTEND_QUERY_MONITOR_IN {
    UINT32 StructSize; // = sizeof(EXTEND_QUERY_MONITOR_IN) = 8
    UINT32 MonitorId;
} EXTEND_QUERY_MONITOR_IN;

// EXTEND_QUERY_MONITOR_OUT.Flags bits.
#define EXTEND_MONITOR_FLAG_PRESENT 0x1u // monitor created and arrival reported
#define EXTEND_MONITOR_FLAG_SURFACE 0x2u // shared texture published; name/luid/size valid
#define EXTEND_MONITOR_FLAG_FRAME   0x4u // at least one frame has been published

// IOCTL_EXTEND_QUERY_MONITOR output.
typedef struct _EXTEND_QUERY_MONITOR_OUT {
    UINT32 StructSize;        // = sizeof(EXTEND_QUERY_MONITOR_OUT) = 172
    UINT32 MonitorId;
    UINT32 Flags;             // EXTEND_MONITOR_FLAG_*
    UINT32 SurfaceGeneration; // bumps whenever a new shared texture replaces the old
                              // one (swap-chain reassign, mode change, abandoned
                              // keyed mutex); 0 until the first. A client holding an
                              // older generation must close it and reopen by name.
    UINT32 RenderLuidLow;     // render-adapter LUID the shared texture lives on
    INT32  RenderLuidHigh;
    UINT32 Width;             // shared-texture width in pixels (0 until SURFACE)
    UINT32 Height;            // shared-texture height in pixels
    UINT32 DxgiFormat;        // DXGI_FORMAT of the shared texture
    UINT32 FrameCounterLow;   // 64-bit count of frames published, monotonic per
    UINT32 FrameCounterHigh;  // monitor across generations; split for alignment
    WCHAR  TextureName[EXTEND_TEXTURE_NAME_LEN]; // NUL-terminated, unique per generation
} EXTEND_QUERY_MONITOR_OUT;

#include <poppack.h>

#ifdef __cplusplus
}
#endif
