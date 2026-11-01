/*++

Copyright (c) Microsoft Corporation

    This file is derived from Microsoft's IddSampleDriver
    (Windows-driver-samples, video/IndirectDisplay/IddSampleDriver/Driver.cpp),
    licensed under the Microsoft Public License (MS-PL). The MS-PL notice and a
    pointer to the full license are kept in native/windows/driver/README.md and
    native/windows/driver/LICENSE-MS-PL.

    extend.computer modifications:
      * The adapter starts with ZERO monitors. Monitors are created and removed
        only by a client over a custom device interface, via the IOCTL contract
        in Protocol.h, each with the client's mode list and EDID identity.
      * Every open is vetted (EvtDeviceFileCreate): callers below Medium
        integrity, and handles whose QoS hides the caller's token, are
        refused, which the device's DACL alone does not do.
      * Each monitor is owned by the file handle that created it. Only that
        handle may query or remove it, and when it closes (EvtFileCleanup) the
        monitor departs, so a crashed helper can never leave a stray monitor,
        even one whose ADD was still in flight.
      * Monitors depart the way IddCx tears them down itself:
        IddCxMonitorDeparture unassigns the swap chain (our processor stops and
        deletes it) and then deletes the monitor. Every departure runs
        serialised with IddCx's sequential I/O-control queue, which also
        carries the kernel's new-swap-chain messages, so a departure never
        overlaps Windows assigning a swap chain to the same monitor; a handle
        closed right after ADD used to crash the host there. See "Monitor
        teardown" in native/windows/driver/README.md.
      * The swap-chain processor copies each acquired desktop surface into a
        per-monitor keyed-mutex shared D3D11 texture whose DACL admits only the
        creating client's logon SID (read through request impersonation), and
        exposes a frame counter and a surface generation.

Environment:

    Windows User-Mode Driver Framework 2 / IddCx

--*/

#include "Driver.h"
#include <initguid.h>
#include <sddl.h>
#include <stdio.h>

#ifdef EXTEND_HAVE_VERSION_H
#include "ExtendVersion.h" // written per build by scripts/windows/driver.ps1 from DriverVer
#endif

using namespace std;
using namespace ExtendComputer;
using namespace Microsoft::WRL;

#ifndef EXTEND_VER_MAJOR
#define EXTEND_VER_MAJOR 0
#define EXTEND_VER_MINOR 0
#define EXTEND_VER_BUILD 0
#define EXTEND_VER_REVISION 0
#endif

// Device-interface class the client opens. {E053DB0E-1431-463B-ABDC-0CB768D6E649}
DEFINE_GUID(GUID_DEVINTERFACE_EXTEND_DISPLAY,
    0xe053db0e, 0x1431, 0x463b, 0xab, 0xdc, 0x0c, 0xb7, 0x68, 0xd6, 0xe6, 0x49);

// Shared-texture security descriptor, filled in with the creating client's
// logon SID (its user SID if the token has none). SYSTEM keeps full control.
// The owner (WUDFHost's LOCAL SERVICE identity) gets only READ_CONTROL, so other
// LOCAL SERVICE processes cannot rewrite the DACL. The client gets exactly what
// OpenSharedResourceByName asks for, DXGI_SHARED_RESOURCE_READ|WRITE. The Medium
// mandatory label with no-read-up/no-write-up keeps low-integrity processes out
// even when they carry that SID. The texture name is not a secret; this
// descriptor is the gate. See native/windows/driver/README.md.
static const wchar_t* const kTextureSddlFormat =
    L"D:P(A;;GA;;;SY)(A;;RC;;;OW)(A;;0x80000001;;;%s)S:(ML;;NWNR;;;ME)";

// Keyed-mutex keys (the Duet handoff): the driver writes under key 0 and
// releases key 1; the client reads under key 1 and releases key 0.
static const UINT64 kKeyFree = 0;
static const UINT64 kKeyFrame = 1;

// There is exactly one adapter devnode for this driver (the installer creates a
// single Root\ExtendComputerDisplay node), so a single global reaches the device
// context from the IddCx ParseMonitorDescription callback, which carries no
// object handle. Set in InitAdapter, cleared in the destructor.
static IndirectDeviceContext* g_pDeviceContext = nullptr;

static void ExtendLog(const char* fmt, ...)
{
    char buf[256];
    va_list ap;
    va_start(ap, fmt);
    _vsnprintf_s(buf, sizeof(buf), _TRUNCATE, fmt, ap);
    va_end(ap);
    OutputDebugStringA(buf);
}

#pragma region helpers

static inline void FillSignalInfo(DISPLAYCONFIG_VIDEO_SIGNAL_INFO& Mode, DWORD Width, DWORD Height, DWORD VSync, bool bMonitorMode)
{
    Mode.totalSize.cx = Mode.activeSize.cx = Width;
    Mode.totalSize.cy = Mode.activeSize.cy = Height;

    Mode.AdditionalSignalInfo.vSyncFreqDivider = bMonitorMode ? 0 : 1;
    Mode.AdditionalSignalInfo.videoStandard = 255;

    Mode.vSyncFreq.Numerator = VSync;
    Mode.vSyncFreq.Denominator = 1;
    Mode.hSyncFreq.Numerator = VSync * Height;
    Mode.hSyncFreq.Denominator = 1;

    Mode.scanLineOrdering = DISPLAYCONFIG_SCANLINE_ORDERING_PROGRESSIVE;

    Mode.pixelRate = ((UINT64) VSync) * ((UINT64) Width) * ((UINT64) Height);
}

static IDDCX_MONITOR_MODE CreateIddCxMonitorMode(DWORD Width, DWORD Height, DWORD VSync, IDDCX_MONITOR_MODE_ORIGIN Origin = IDDCX_MONITOR_MODE_ORIGIN_DRIVER)
{
    IDDCX_MONITOR_MODE Mode = {};
    Mode.Size = sizeof(Mode);
    Mode.Origin = Origin;
    FillSignalInfo(Mode.MonitorVideoSignalInfo, Width, Height, VSync, true);
    return Mode;
}

static IDDCX_TARGET_MODE CreateIddCxTargetMode(DWORD Width, DWORD Height, DWORD VSync)
{
    IDDCX_TARGET_MODE Mode = {};
    Mode.Size = sizeof(Mode);
    FillSignalInfo(Mode.TargetVideoSignalInfo.targetVideoSignalInfo, Width, Height, VSync, false);
    return Mode;
}

static bool ValidMode(UINT Width, UINT Height, UINT RefreshHz)
{
    return Width >= EXTEND_MIN_DIMENSION && Width <= EXTEND_MAX_DIMENSION &&
        Height >= EXTEND_MIN_DIMENSION && Height <= EXTEND_MAX_DIMENSION &&
        RefreshHz >= 1 && RefreshHz <= EXTEND_MAX_REFRESH_HZ;
}

#pragma endregion

extern "C" DRIVER_INITIALIZE DriverEntry;

EVT_WDF_DRIVER_DEVICE_ADD ExtendDeviceAdd;
EVT_WDF_DEVICE_D0_ENTRY ExtendDeviceD0Entry;
EVT_WDF_DEVICE_FILE_CREATE ExtendFileCreate;
EVT_WDF_FILE_CLEANUP ExtendFileCleanup;
EVT_WDF_REQUEST_IMPERSONATE ExtendReadClientToken;

EVT_IDD_CX_ADAPTER_INIT_FINISHED ExtendAdapterInitFinished;
EVT_IDD_CX_ADAPTER_COMMIT_MODES ExtendAdapterCommitModes;
EVT_IDD_CX_DEVICE_IO_CONTROL ExtendDeviceIoControl;

EVT_IDD_CX_PARSE_MONITOR_DESCRIPTION ExtendParseMonitorDescription;
EVT_IDD_CX_MONITOR_GET_DEFAULT_DESCRIPTION_MODES ExtendMonitorGetDefaultModes;
EVT_IDD_CX_MONITOR_QUERY_TARGET_MODES ExtendMonitorQueryModes;

EVT_IDD_CX_MONITOR_ASSIGN_SWAPCHAIN ExtendMonitorAssignSwapChain;
EVT_IDD_CX_MONITOR_UNASSIGN_SWAPCHAIN ExtendMonitorUnassignSwapChain;

struct IndirectDeviceContextWrapper
{
    IndirectDeviceContext* pContext;
    void Cleanup() { delete pContext; pContext = nullptr; }
};

struct IndirectMonitorContextWrapper
{
    IndirectMonitorContext* pContext;
    void Cleanup() { delete pContext; pContext = nullptr; }
};

// Per-handle state. Closed is set under the device lock when the handle's
// cleanup runs. The I/O manager sends cleanup as soon as the last handle
// closes, even with an ADD still pending, and the ADD can reach the driver
// after cleanup has run; it checks Closed so it never leaves a monitor that
// nothing would depart.
struct ExtendFileContext
{
    bool Closed;
};

WDF_DECLARE_CONTEXT_TYPE(IndirectDeviceContextWrapper);
WDF_DECLARE_CONTEXT_TYPE(IndirectMonitorContextWrapper);
WDF_DECLARE_CONTEXT_TYPE(ExtendFileContext);

static bool FileClosed(WDFFILEOBJECT File)
{
    auto* f = WdfObjectGet_ExtendFileContext(File);
    return !f || f->Closed;
}

// Reports a monitor's departure. IddCxMonitorDeparture tells the OS, calls
// EvtIddCxMonitorUnassignSwapChain if a swap chain is assigned (the processor
// stops and deletes it, as for any unassign), deletes it itself if it is still
// there, and then deletes the monitor object, whose cleanup frees our context.
// Nothing may stop or delete the swap chain beforehand: a swap chain the
// driver drops on a monitor that is still present makes Windows assign a new
// one at once, and that assignment races the departure inside IddCx.
//
// The caller runs it serialised with IddCx's I/O-control queue (from one of
// our IOCTLs, or with that queue stopped; see RemoveMonitorsForFile), and
// without m_Lock, since departure re-enters IddCx callbacks that take it.
static NTSTATUS DepartMonitor(IDDCX_MONITOR Monitor, UINT32 MonitorId)
{
    NTSTATUS status = IddCxMonitorDeparture(Monitor);
    ExtendLog("extend: departed monitor id=%u 0x%08x (thread %u)\n", MonitorId, status, GetCurrentThreadId());
    return status;
}

extern "C" BOOL WINAPI DllMain(HINSTANCE, UINT, LPVOID) { return TRUE; }

_Use_decl_annotations_
extern "C" NTSTATUS DriverEntry(PDRIVER_OBJECT pDriverObject, PUNICODE_STRING pRegistryPath)
{
    WDF_DRIVER_CONFIG Config;
    WDF_OBJECT_ATTRIBUTES Attributes;
    WDF_OBJECT_ATTRIBUTES_INIT(&Attributes);
    WDF_DRIVER_CONFIG_INIT(&Config, ExtendDeviceAdd);

    return WdfDriverCreate(pDriverObject, pRegistryPath, &Attributes, &Config, WDF_NO_HANDLE);
}

_Use_decl_annotations_
NTSTATUS ExtendDeviceAdd(WDFDRIVER Driver, PWDFDEVICE_INIT pDeviceInit)
{
    UNREFERENCED_PARAMETER(Driver);
    NTSTATUS Status;

    // Vet every open (see ExtendFileCreate) and track file objects, so a
    // closing handle departs its monitors.
    WDF_FILEOBJECT_CONFIG FileConfig;
    WDF_FILEOBJECT_CONFIG_INIT(&FileConfig, ExtendFileCreate, WDF_NO_EVENT_CALLBACK, ExtendFileCleanup);
    WDF_OBJECT_ATTRIBUTES FileAttr;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&FileAttr, ExtendFileContext);
    WdfDeviceInitSetFileObjectConfig(pDeviceInit, &FileConfig, &FileAttr);

    WDF_PNPPOWER_EVENT_CALLBACKS PnpPowerCallbacks;
    WDF_PNPPOWER_EVENT_CALLBACKS_INIT(&PnpPowerCallbacks);
    PnpPowerCallbacks.EvtDeviceD0Entry = ExtendDeviceD0Entry;
    WdfDeviceInitSetPnpPowerEventCallbacks(pDeviceInit, &PnpPowerCallbacks);

    IDD_CX_CLIENT_CONFIG IddConfig;
    IDD_CX_CLIENT_CONFIG_INIT(&IddConfig);

    IddConfig.EvtIddCxDeviceIoControl = ExtendDeviceIoControl;
    IddConfig.EvtIddCxAdapterInitFinished = ExtendAdapterInitFinished;
    IddConfig.EvtIddCxParseMonitorDescription = ExtendParseMonitorDescription;
    IddConfig.EvtIddCxMonitorGetDefaultDescriptionModes = ExtendMonitorGetDefaultModes;
    IddConfig.EvtIddCxMonitorQueryTargetModes = ExtendMonitorQueryModes;
    IddConfig.EvtIddCxAdapterCommitModes = ExtendAdapterCommitModes;
    IddConfig.EvtIddCxMonitorAssignSwapChain = ExtendMonitorAssignSwapChain;
    IddConfig.EvtIddCxMonitorUnassignSwapChain = ExtendMonitorUnassignSwapChain;

    Status = IddCxDeviceInitConfig(pDeviceInit, &IddConfig);
    if (!NT_SUCCESS(Status)) return Status;

    WDF_OBJECT_ATTRIBUTES Attr;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&Attr, IndirectDeviceContextWrapper);
    Attr.EvtCleanupCallback = [](WDFOBJECT Object)
    {
        auto* pContext = WdfObjectGet_IndirectDeviceContextWrapper(Object);
        if (pContext) pContext->Cleanup();
    };

    WDFDEVICE Device = nullptr;
    Status = WdfDeviceCreate(&pDeviceInit, &Attr, &Device);
    if (!NT_SUCCESS(Status)) return Status;

    Status = IddCxDeviceInitialize(Device);
    if (!NT_SUCCESS(Status)) return Status;

    // Publish the custom device interface the client opens. Its security comes
    // from the INF .hw Security SDDL on the device object, and ExtendFileCreate
    // refuses callers below Medium integrity, which that DACL lets in.
    Status = WdfDeviceCreateDeviceInterface(Device, &GUID_DEVINTERFACE_EXTEND_DISPLAY, nullptr);
    if (!NT_SUCCESS(Status)) return Status;

    auto* pContext = WdfObjectGet_IndirectDeviceContextWrapper(Device);
    pContext->pContext = new IndirectDeviceContext(Device);

    return Status;
}

_Use_decl_annotations_
NTSTATUS ExtendDeviceD0Entry(WDFDEVICE Device, WDF_POWER_DEVICE_STATE PreviousState)
{
    UNREFERENCED_PARAMETER(PreviousState);
    auto* pContext = WdfObjectGet_IndirectDeviceContextWrapper(Device);
    pContext->pContext->InitAdapter();
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
VOID ExtendFileCleanup(WDFFILEOBJECT FileObject)
{
    WDFDEVICE Device = WdfFileObjectGetDevice(FileObject);
    auto* wrapper = WdfObjectGet_IndirectDeviceContextWrapper(Device);
    if (wrapper && wrapper->pContext)
    {
        wrapper->pContext->RemoveMonitorsForFile(FileObject);
    }
}

// What the driver needs from a caller's token, read while WdfRequestImpersonate
// runs ExtendReadClientToken: its integrity level (every open is vetted) and
// the SID an ADD request's monitor texture is scoped to.
struct ClientToken
{
    NTSTATUS Status = STATUS_BAD_IMPERSONATION_LEVEL;
    DWORD IntegrityRid = 0;   // SECURITY_MANDATORY_*_RID
    wchar_t Sid[192] = {};
};

static bool QueryToken(HANDLE Token, TOKEN_INFORMATION_CLASS Class, vector<BYTE>& Out)
{
    DWORD size = 0;
    GetTokenInformation(Token, Class, nullptr, 0, &size);
    if (size == 0) return false;
    Out.resize(size);
    return GetTokenInformation(Token, Class, Out.data(), size, &size) != FALSE;
}

_Use_decl_annotations_
VOID ExtendReadClientToken(WDFREQUEST Request, PVOID Context)
{
    UNREFERENCED_PARAMETER(Request);
    auto* out = static_cast<ClientToken*>(Context);

    // Identification-level impersonation cannot open objects as the client, so
    // the thread token is opened with the driver's own identity (OpenAsSelf).
    HANDLE token = nullptr;
    if (!OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, TRUE, &token))
    {
        ExtendLog("extend: OpenThreadToken %u\n", GetLastError());
        return;
    }

    vector<BYTE> label, groups, user;
    if (QueryToken(token, TokenIntegrityLevel, label))
    {
        PSID il = reinterpret_cast<TOKEN_MANDATORY_LABEL*>(label.data())->Label.Sid;
        UCHAR n = *GetSidSubAuthorityCount(il);
        if (n > 0) out->IntegrityRid = *GetSidSubAuthority(il, n - 1);
    }

    // Prefer the logon SID (S-1-5-5-X-Y): it admits only processes of this one
    // logon session, not the same user signed in elsewhere.
    PSID sid = nullptr;
    if (QueryToken(token, TokenGroups, groups))
    {
        auto* g = reinterpret_cast<TOKEN_GROUPS*>(groups.data());
        for (DWORD i = 0; i < g->GroupCount && !sid; i++)
        {
            if ((g->Groups[i].Attributes & SE_GROUP_LOGON_ID) == SE_GROUP_LOGON_ID) sid = g->Groups[i].Sid;
        }
    }
    if (!sid && QueryToken(token, TokenUser, user))
    {
        sid = reinterpret_cast<TOKEN_USER*>(user.data())->User.Sid;
    }

    LPWSTR text = nullptr;
    if (sid && ConvertSidToStringSidW(sid, &text))
    {
        if (wcscpy_s(out->Sid, text) == 0) out->Status = STATUS_SUCCESS;
        LocalFree(text);
    }
    CloseHandle(token);
}

// Every open of the device. The INF's device security admits SYSTEM,
// administrators and interactive users, but the device object carries no
// mandatory label (PnP keeps only the DACL of the INF's Security SDDL), so
// the I/O manager lets a Low-integrity process of the signed-in user (a
// sandboxed browser or document viewer) open it. This refuses every open
// below Medium integrity, so such a process can neither create monitors nor
// learn anything from the driver. Opens from kernel mode (the display
// kernel's own channel to IddCx) pass. A handle opened with anonymous QoS
// hides the caller's token, so it is refused as well.
_Use_decl_annotations_
VOID ExtendFileCreate(WDFDEVICE Device, WDFREQUEST Request, WDFFILEOBJECT FileObject)
{
    UNREFERENCED_PARAMETER(Device);
    UNREFERENCED_PARAMETER(FileObject);
    if (WdfRequestGetRequestorMode(Request) == KernelMode)
    {
        WdfRequestComplete(Request, STATUS_SUCCESS);
        return;
    }
    ClientToken client;
    NTSTATUS status = WdfRequestImpersonate(Request, SecurityIdentification, ExtendReadClientToken, &client);
    if (!NT_SUCCESS(status) || !NT_SUCCESS(client.Status) || client.IntegrityRid < SECURITY_MANDATORY_MEDIUM_RID)
    {
        ExtendLog("extend: open refused (impersonation 0x%08x/0x%08x, integrity 0x%x)\n", status, client.Status,
            client.IntegrityRid);
        WdfRequestComplete(Request, STATUS_ACCESS_DENIED);
        return;
    }
    WdfRequestComplete(Request, STATUS_SUCCESS);
}

#pragma region Direct3DDevice

Direct3DDevice::Direct3DDevice(LUID AdapterLuid) : AdapterLuid(AdapterLuid) {}
Direct3DDevice::Direct3DDevice() { AdapterLuid = LUID{}; }

HRESULT Direct3DDevice::Init()
{
    HRESULT hr = CreateDXGIFactory2(0, IID_PPV_ARGS(&DxgiFactory));
    if (FAILED(hr)) return hr;

    hr = DxgiFactory->EnumAdapterByLuid(AdapterLuid, IID_PPV_ARGS(&Adapter));
    if (FAILED(hr)) return hr;

    hr = D3D11CreateDevice(Adapter.Get(), D3D_DRIVER_TYPE_UNKNOWN, nullptr, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        nullptr, 0, D3D11_SDK_VERSION, &Device, nullptr, &DeviceContext);
    return hr;
}

#pragma endregion

#pragma region SwapChainProcessor

SwapChainProcessor::SwapChainProcessor(IDDCX_SWAPCHAIN hSwapChain, shared_ptr<Direct3DDevice> Device,
    HANDLE NewFrameEvent, IndirectMonitorContext* Monitor)
    : m_hSwapChain(hSwapChain), m_Device(Device), m_hAvailableBufferEvent(NewFrameEvent), m_Monitor(Monitor)
{
    // Manual reset: the event stays set for the per-frame check in RunCore.
    m_hTerminateEvent.Attach(CreateEvent(nullptr, TRUE, FALSE, nullptr));
    m_hThread.Attach(CreateThread(nullptr, 0, RunThread, this, 0, nullptr));
}

SwapChainProcessor::~SwapChainProcessor()
{
    // The thread sees the terminate event, leaves RunCore and deletes the swap
    // chain (unless orphaned) before this returns. The unassign callback runs
    // this under IddCx's DDI watchdog, so the thread checks for the stop on
    // every frame.
    SetEvent(m_hTerminateEvent.Get());
    if (m_hThread.Get())
    {
        WaitForSingleObject(m_hThread.Get(), INFINITE);
    }
    ReleaseSharedTexture();
}

DWORD CALLBACK SwapChainProcessor::RunThread(LPVOID Argument)
{
    reinterpret_cast<SwapChainProcessor*>(Argument)->Run();
    return 0;
}

void SwapChainProcessor::Run()
{
    DWORD AvTask = 0;
    HANDLE AvTaskHandle = AvSetMmThreadCharacteristicsW(L"Distribution", &AvTask);

    RunCore();

    // The driver owns an accepted swap chain and deletes it when it stops
    // processing: on an unassign (IddCx then finds it gone and skips its own
    // delete), or after an error, which makes the OS assign a new one. An
    // orphaned one is being deleted with its monitor already; touching it
    // again would fault.
    bool stopped = WaitForSingleObject(m_hTerminateEvent.Get(), 0) == WAIT_OBJECT_0;
    ExtendLog("extend: monitor %u processor exits (%s)%s\n", m_Monitor->MonitorId(), stopped ? "stopped" : "swap chain failed",
        m_SwapChainOrphaned ? ", orphaned" : "");
    if (!m_SwapChainOrphaned) WdfObjectDelete((WDFOBJECT) m_hSwapChain);
    m_hSwapChain = nullptr;

    AvRevertMmThreadCharacteristics(AvTaskHandle);
}

bool SwapChainProcessor::EnsureSharedTexture(const D3D11_TEXTURE2D_DESC& Source)
{
    bool sameShape = m_LatestTexture && m_Desc.Width == Source.Width && m_Desc.Height == Source.Height &&
        m_Desc.Format == Source.Format;
    if (m_SharedTexture && sameShape) return true;

    if (!sameShape)
    {
        // First frame, or the surface changed shape: replace both textures.
        ReleaseSharedTexture();
        m_LatestTexture.Reset();
        m_Pending = false;
    }
    if (GetTickCount64() < m_RetryAfter) return false;

    D3D11_TEXTURE2D_DESC desc = {};
    desc.Width = Source.Width;
    desc.Height = Source.Height;
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    desc.Format = Source.Format;
    desc.SampleDesc.Count = 1;
    desc.SampleDesc.Quality = 0;
    desc.Usage = D3D11_USAGE_DEFAULT;

    HRESULT hr = S_OK;
    if (!m_LatestTexture)
    {
        hr = m_Device->Device->CreateTexture2D(&desc, nullptr, &m_LatestTexture);
        if (FAILED(hr))
        {
            ExtendLog("extend: monitor %u latest-frame texture 0x%08x\n", m_Monitor->MonitorId(), hr);
            m_RetryAfter = GetTickCount64() + 500;
            return false;
        }
        m_Desc = desc;
    }

    desc.BindFlags = D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE;
    desc.MiscFlags = D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX | D3D11_RESOURCE_MISC_SHARED_NTHANDLE;

    MonitorSurface& surface = m_Monitor->Surface();
    EnterCriticalSection(&surface.Lock);
    UINT32 generation = surface.Generation + 1;
    LeaveCriticalSection(&surface.Lock);

    // A fresh name per generation, so a client can never mistake a replaced
    // texture for the one it holds.
    wchar_t name[EXTEND_TEXTURE_NAME_LEN];
    swprintf_s(name, L"Global\\ExtendComputerDisplay-%u-%u-%08x", m_Monitor->MonitorId(), generation, m_Monitor->Nonce());

    ComPtr<ID3D11Texture2D> texture;
    ComPtr<IDXGIResource1> resource;
    ComPtr<IDXGIKeyedMutex> mutex;
    HANDLE handle = nullptr;
    PSECURITY_DESCRIPTOR sd = nullptr;

    hr = m_Device->Device->CreateTexture2D(&desc, nullptr, &texture);
    if (SUCCEEDED(hr)) hr = texture.As(&resource);
    if (SUCCEEDED(hr)) hr = texture.As(&mutex);
    if (SUCCEEDED(hr) && !ConvertStringSecurityDescriptorToSecurityDescriptorW(m_Monitor->TextureSddl(), SDDL_REVISION_1, &sd, nullptr))
    {
        hr = HRESULT_FROM_WIN32(GetLastError());
    }
    if (SUCCEEDED(hr))
    {
        SECURITY_ATTRIBUTES sa = { sizeof(sa), sd, FALSE };
        hr = resource->CreateSharedHandle(&sa, DXGI_SHARED_RESOURCE_READ | DXGI_SHARED_RESOURCE_WRITE, name, &handle);
    }
    if (sd) LocalFree(sd);
    if (FAILED(hr))
    {
        ExtendLog("extend: monitor %u shared texture 0x%08x\n", m_Monitor->MonitorId(), hr);
        m_RetryAfter = GetTickCount64() + 500;
        return false;
    }

    m_SharedTexture = texture;
    m_KeyedMutex = mutex;
    m_SharedHandle = handle;
    m_Generation = generation;

    EnterCriticalSection(&surface.Lock);
    surface.Published = true;
    surface.Generation = generation;
    surface.RenderLuid = m_Device->AdapterLuid;
    surface.Width = desc.Width;
    surface.Height = desc.Height;
    surface.Format = (UINT) desc.Format;
    wcscpy_s(surface.Name, name);
    LeaveCriticalSection(&surface.Lock);

    ExtendLog("extend: monitor %u surface generation %u %ux%u\n", m_Monitor->MonitorId(), generation, desc.Width, desc.Height);
    return true;
}

void SwapChainProcessor::ReleaseSharedTexture()
{
    if (!m_SharedTexture) return;

    // Withdraw the published surface only if it is still this texture: a
    // retired processor (see AssignSwapChain) may finish after its successor
    // published a newer one.
    MonitorSurface& surface = m_Monitor->Surface();
    EnterCriticalSection(&surface.Lock);
    if (surface.Generation == m_Generation) surface.Published = false;
    LeaveCriticalSection(&surface.Lock);

    // Take the keyed mutex if it is free and drop the texture without releasing
    // it. That abandons the mutex for a client still holding this texture, whose
    // next AcquireSync returns WAIT_ABANDONED: a prompt to query and reopen. (A
    // client reading at this moment keeps its frame and then only times out;
    // the generation in QUERY_MONITOR is the authoritative signal.)
    AcquireForWrite();
    m_KeyedMutex.Reset();
    m_SharedTexture.Reset();
    if (m_SharedHandle)
    {
        CloseHandle(m_SharedHandle);
        m_SharedHandle = nullptr;
    }
    m_Device->DeviceContext->Flush();
}

HRESULT SwapChainProcessor::AcquireForWrite()
{
    // Key 0: the client released the last frame. Key 1: the client never picked
    // the last frame up, so take it back and replace it with the newer one.
    // Never wait; desktop production must not block on the client.
    HRESULT hr = m_KeyedMutex->AcquireSync(kKeyFree, 0);
    if (hr == (HRESULT) WAIT_TIMEOUT) hr = m_KeyedMutex->AcquireSync(kKeyFrame, 0);
    return hr;
}

void SwapChainProcessor::Publish(ID3D11Texture2D* Source)
{
    m_Device->DeviceContext->CopyResource(m_SharedTexture.Get(), Source);
    m_Device->DeviceContext->Flush();
    m_KeyedMutex->ReleaseSync(kKeyFrame);
    m_Pending = false;

    MonitorSurface& surface = m_Monitor->Surface();
    EnterCriticalSection(&surface.Lock);
    surface.FrameCounter++;
    surface.HadFrame = true;
    LeaveCriticalSection(&surface.Lock);
}

void SwapChainProcessor::PublishFrame(ID3D11Texture2D* Source)
{
    D3D11_TEXTURE2D_DESC desc = {};
    Source->GetDesc(&desc);

    // The second pass runs on a fresh texture after the first found the keyed
    // mutex abandoned, so this frame still reaches the client.
    for (int attempt = 0; attempt < 2; attempt++)
    {
        if (!EnsureSharedTexture(desc)) return;

        HRESULT hr = AcquireForWrite();
        if (hr == S_OK)
        {
            Publish(Source);
            return;
        }
        if (hr == (HRESULT) WAIT_TIMEOUT)
        {
            // The client is reading. Keep the newest frame privately;
            // PublishPending hands it over once the client releases.
            m_Device->DeviceContext->CopyResource(m_LatestTexture.Get(), Source);
            m_Pending = true;
            return;
        }
        if (!RecoverFromMutexFailure(hr)) return;
    }
}

void SwapChainProcessor::PublishPending()
{
    if (!m_Pending || !m_LatestTexture) return;
    if (!EnsureSharedTexture(m_Desc)) return;

    HRESULT hr = AcquireForWrite();
    if (hr == S_OK)
    {
        Publish(m_LatestTexture.Get());
    }
    else if (hr != (HRESULT) WAIT_TIMEOUT)
    {
        // The next pass publishes the held frame into the replacement texture.
        RecoverFromMutexFailure(hr);
    }
}

bool SwapChainProcessor::RecoverFromMutexFailure(HRESULT hr)
{
    HRESULT removed = m_Device->Device->GetDeviceRemovedReason();
    if (FAILED(removed))
    {
        // The render device is gone; ending the processor deletes the swap
        // chain and IddCx assigns a new one on a working device.
        ExtendLog("extend: monitor %u render device lost 0x%08x\n", m_Monitor->MonitorId(), removed);
        m_DeviceLost = true;
        return false;
    }

    // WAIT_ABANDONED: a client's device went away while holding the frame (a
    // crash or a device reset). E_FAIL and other errors leave the mutex unusable
    // too. Replace the texture with a new generation, which clients see in
    // QUERY_MONITOR and reopen by its new name.
    ExtendLog("extend: monitor %u keyed mutex 0x%08x, replacing the shared texture\n", m_Monitor->MonitorId(), hr);
    ReleaseSharedTexture();
    return true;
}

void SwapChainProcessor::RunCore()
{
    ComPtr<IDXGIDevice> DxgiDevice;
    HRESULT hr = m_Device->Device.As(&DxgiDevice);
    if (FAILED(hr)) return;

    IDARG_IN_SWAPCHAINSETDEVICE SetDevice = {};
    SetDevice.pDevice = DxgiDevice.Get();
    hr = IddCxSwapChainSetDevice(m_hSwapChain, &SetDevice);
    if (FAILED(hr)) return;

    for (;;)
    {
        // A stop request wins even while frames keep arriving; otherwise a
        // busy desktop would keep the unassign callback waiting past IddCx's
        // timeout.
        if (WaitForSingleObject(m_hTerminateEvent.Get(), 0) == WAIT_OBJECT_0) break;

        ComPtr<IDXGIResource> AcquiredBuffer;
        IDARG_OUT_RELEASEANDACQUIREBUFFER Buffer = {};
        hr = IddCxSwapChainReleaseAndAcquireBuffer(m_hSwapChain, &Buffer);

        if (hr == E_PENDING)
        {
            // No new desktop frame. A frame held back while the client was
            // reading goes out now if the client has released the texture.
            PublishPending();
            if (m_DeviceLost) break;

            HANDLE WaitHandles[] = { m_hAvailableBufferEvent, m_hTerminateEvent.Get() };
            DWORD WaitResult = WaitForMultipleObjects(ARRAYSIZE(WaitHandles), WaitHandles, FALSE, 16);
            if (WaitResult == WAIT_OBJECT_0 || WaitResult == WAIT_TIMEOUT) continue;
            else if (WaitResult == WAIT_OBJECT_0 + 1) break;
            else { hr = HRESULT_FROM_WIN32(WaitResult); break; }
        }
        else if (SUCCEEDED(hr))
        {
            AcquiredBuffer.Attach(Buffer.MetaData.pSurface);

            ComPtr<ID3D11Texture2D> Texture;
            if (SUCCEEDED(AcquiredBuffer.As(&Texture)))
            {
                PublishFrame(Texture.Get());
            }

            AcquiredBuffer.Reset();

            hr = IddCxSwapChainFinishedProcessingFrame(m_hSwapChain);
            if (FAILED(hr) || m_DeviceLost) break;
        }
        else
        {
            break; // swap chain abandoned (e.g. DXGI_ERROR_ACCESS_LOST)
        }
    }
}

#pragma endregion

#pragma region IndirectMonitorContext

IndirectMonitorContext::IndirectMonitorContext(IDDCX_MONITOR Monitor, UINT32 MonitorId, const EXTEND_MODE* Modes,
    UINT ModeCount, const wchar_t* TextureSddl)
    : m_Monitor(Monitor), m_MonitorId(MonitorId)
{
    InitializeCriticalSection(&m_ProcessingLock);
    for (UINT i = 0; i < ModeCount && i < ARRAYSIZE(m_Modes); i++) m_Modes[m_ModeCount++] = Modes[i];
    wcscpy_s(m_TextureSddl, TextureSddl);

    // Distinguishes texture names across adapter restarts, which reuse ids.
    FILETIME ft;
    GetSystemTimePreciseAsFileTime(&ft);
    m_Nonce = ft.dwLowDateTime ^ (ft.dwHighDateTime * 2654435761u);
}

IndirectMonitorContext::~IndirectMonitorContext()
{
    // The monitor object is being deleted. After a departure IddCx has already
    // unassigned the swap chain, so no processor is left, or only one whose
    // thread deleted its swap chain and exited. Without a departure (device
    // teardown) WDF deletes the swap chain with the monitor, so a processor
    // still running must not touch it: orphan it, then join.
    EnterCriticalSection(&m_ProcessingLock);
    if (m_ProcessingThread) m_ProcessingThread->OrphanSwapChain();
    m_ProcessingThread.reset();
    m_Retired.clear();
    LeaveCriticalSection(&m_ProcessingLock);
    DeleteCriticalSection(&m_ProcessingLock);
}

void IndirectMonitorContext::AssignSwapChain(IDDCX_SWAPCHAIN SwapChain, LUID RenderAdapter, HANDLE NewFrameEvent)
{
    ExtendLog("extend: monitor %u assign swap chain %p (thread %u)\n", m_MonitorId, SwapChain, GetCurrentThreadId());
    EnterCriticalSection(&m_ProcessingLock);
    // IddCx unassigns the previous swap chain before assigning a new one, so
    // normally no processor is left here. One may be if its swap chain failed:
    // its thread deletes the swap chain itself, which makes the OS assign this
    // one, and the delete takes the monitor lock IddCx holds around this
    // callback. Joining that thread here could deadlock, so it is retired and
    // joined at the next unassign instead (where IddCx holds no lock), or with
    // the monitor. Retired processors that have exited go now.
    if (m_ProcessingThread && m_ProcessingThread->Running()) m_Retired.push_back(std::move(m_ProcessingThread));
    m_ProcessingThread.reset();
    for (auto it = m_Retired.begin(); it != m_Retired.end();)
    {
        if ((*it)->Running()) ++it;
        else it = m_Retired.erase(it);
    }

    auto Device = make_shared<Direct3DDevice>(RenderAdapter);
    HRESULT hr = Device->Init();
    if (FAILED(hr))
    {
        // As the sample does: deleting the swap chain makes the OS assign a
        // new one and try again.
        ExtendLog("extend: monitor %u render device 0x%08x\n", m_MonitorId, hr);
        WdfObjectDelete(SwapChain);
    }
    else
    {
        m_ProcessingThread.reset(new SwapChainProcessor(SwapChain, Device, NewFrameEvent, this));
    }
    LeaveCriticalSection(&m_ProcessingLock);
}

void IndirectMonitorContext::UnassignSwapChain()
{
    // A mode or topology change, or the monitor's departure (IddCx calls this
    // from IddCxMonitorDeparture): stop processing. The thread deletes the
    // swap chain as it exits, before this returns; IddCx then finds it gone.
    // IddCx holds no lock of its own around this call.
    ExtendLog("extend: monitor %u unassign swap chain (thread %u)\n", m_MonitorId, GetCurrentThreadId());
    EnterCriticalSection(&m_ProcessingLock);
    m_ProcessingThread.reset();
    m_Retired.clear();
    LeaveCriticalSection(&m_ProcessingLock);
}

#pragma endregion

#pragma region IndirectDeviceContext

IndirectDeviceContext::IndirectDeviceContext(WDFDEVICE WdfDevice) : m_WdfDevice(WdfDevice)
{
    m_Adapter = {};
    WDF_OBJECT_ATTRIBUTES attr;
    WDF_OBJECT_ATTRIBUTES_INIT(&attr);
    attr.ParentObject = WdfDevice;
    WdfWaitLockCreate(&attr, &m_Lock);
    WdfWaitLockCreate(&attr, &m_DepartLock);
}

IndirectDeviceContext::~IndirectDeviceContext()
{
    if (g_pDeviceContext == this) g_pDeviceContext = nullptr;
}

void IndirectDeviceContext::InitAdapter()
{
    IDDCX_ADAPTER_CAPS AdapterCaps = {};
    AdapterCaps.Size = sizeof(AdapterCaps);
    AdapterCaps.MaxMonitorsSupported = EXTEND_MAX_MONITORS;
    AdapterCaps.EndPointDiagnostics.Size = sizeof(AdapterCaps.EndPointDiagnostics);
    AdapterCaps.EndPointDiagnostics.GammaSupport = IDDCX_FEATURE_IMPLEMENTATION_NONE;
    AdapterCaps.EndPointDiagnostics.TransmissionType = IDDCX_TRANSMISSION_TYPE_WIRED_OTHER;
    AdapterCaps.EndPointDiagnostics.pEndPointFriendlyName = L"extend.computer Display";
    AdapterCaps.EndPointDiagnostics.pEndPointManufacturerName = L"extend.computer";
    AdapterCaps.EndPointDiagnostics.pEndPointModelName = L"extend.computer Virtual Display";

    IDDCX_ENDPOINT_VERSION Version = {};
    Version.Size = sizeof(Version);
    Version.MajorVer = 1;
    AdapterCaps.EndPointDiagnostics.pFirmwareVersion = &Version;
    AdapterCaps.EndPointDiagnostics.pHardwareVersion = &Version;

    WDF_OBJECT_ATTRIBUTES Attr;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&Attr, IndirectDeviceContextWrapper);

    IDARG_IN_ADAPTER_INIT AdapterInit = {};
    AdapterInit.WdfDevice = m_WdfDevice;
    AdapterInit.pCaps = &AdapterCaps;
    AdapterInit.ObjectAttributes = &Attr;

    IDARG_OUT_ADAPTER_INIT AdapterInitOut;
    NTSTATUS Status = IddCxAdapterInitAsync(&AdapterInit, &AdapterInitOut);

    if (NT_SUCCESS(Status))
    {
        m_Adapter = AdapterInitOut.AdapterObject;
        auto* pContext = WdfObjectGet_IndirectDeviceContextWrapper(AdapterInitOut.AdapterObject);
        pContext->pContext = this;

        IDARG_OUT_GETVERSION ver = {};
        if (NT_SUCCESS(IddCxGetVersion(&ver))) m_IddCxVersion = ver.IddCxVersion;

        g_pDeviceContext = this;
    }
}

NTSTATUS IndirectDeviceContext::GetInfo(EXTEND_INFO* Out)
{
    Out->StructSize = sizeof(EXTEND_INFO);
    Out->ProtocolVersion = EXTEND_PROTOCOL_VERSION;
    Out->DriverVersionHigh = ((UINT32) EXTEND_VER_MAJOR << 16) | (UINT32) EXTEND_VER_MINOR;
    Out->DriverVersionLow = ((UINT32) EXTEND_VER_BUILD << 16) | (UINT32) EXTEND_VER_REVISION;
    Out->IddCxVersion = m_IddCxVersion;
    Out->MaxMonitors = EXTEND_MAX_MONITORS;

    WdfWaitLockAcquire(m_Lock, NULL);
    UINT active = 0;
    for (auto& s : m_Slots) if (s.State != SlotState::Free) active++;
    Out->ActiveMonitors = active;
    WdfWaitLockRelease(m_Lock);
    return STATUS_SUCCESS;
}

NTSTATUS IndirectDeviceContext::AddMonitor(WDFFILEOBJECT Owner, const wchar_t* OwnerSid,
    const EXTEND_ADD_MONITOR_IN* In, EXTEND_ADD_MONITOR_OUT* Out)
{
    if (In->StructSize != sizeof(*In)) return STATUS_INVALID_PARAMETER;
    if (In->Version != EXTEND_PROTOCOL_VERSION) return STATUS_REVISION_MISMATCH;
    if (!ValidMode(In->Width, In->Height, In->RefreshHz)) return STATUS_INVALID_PARAMETER;
    if (In->FallbackModeCount > EXTEND_MAX_MODES) return STATUS_INVALID_PARAMETER;

    // The requested mode first (preferred), then the distinct fallbacks.
    EXTEND_MODE modes[EXTEND_MAX_MODES + 1];
    UINT modeCount = 0;
    modes[modeCount++] = { In->Width, In->Height, In->RefreshHz };
    for (UINT k = 0; k < In->FallbackModeCount; k++)
    {
        const EXTEND_MODE& m = In->FallbackModes[k];
        if (!ValidMode(m.Width, m.Height, m.RefreshHz)) return STATUS_INVALID_PARAMETER;
        bool dup = false;
        for (UINT j = 0; j < modeCount && !dup; j++)
            dup = modes[j].Width == m.Width && modes[j].Height == m.Height && modes[j].RefreshHz == m.RefreshHz;
        if (!dup) modes[modeCount++] = m;
    }

    wchar_t sddl[EXTEND_SDDL_LEN];
    if (swprintf_s(sddl, kTextureSddlFormat, OwnerSid) < 0) return STATUS_INVALID_PARAMETER;

    // The EDID serial is the client's stable seed, so the same client device
    // maps to the same Windows display-configuration entry every session.
    BYTE edid[EXTEND_EDID_SIZE];
    ExtendBuildEdid(edid, In->Width, In->Height, In->RefreshHz, In->PhysicalWidthMm, In->PhysicalHeightMm, In->SerialSeed);

    WdfWaitLockAcquire(m_Lock, NULL);

    // The handle's cleanup already ran (it can overtake a pending ADD), so no
    // cleanup would ever depart this monitor.
    if (FileClosed(Owner)) { WdfWaitLockRelease(m_Lock); return STATUS_CANCELLED; }

    int idx = -1;
    for (UINT i = 0; i < EXTEND_MAX_MONITORS; i++)
    {
        if (m_Slots[i].State != SlotState::Free && m_Slots[i].SerialSeed == In->SerialSeed)
        {
            // Two monitors with one EDID would be indistinguishable to Windows.
            WdfWaitLockRelease(m_Lock);
            return STATUS_OBJECT_NAME_COLLISION;
        }
        if (idx < 0 && m_Slots[i].State == SlotState::Free) idx = (int) i;
    }
    if (idx < 0) { WdfWaitLockRelease(m_Lock); return STATUS_INSUFFICIENT_RESOURCES; }

    // The slot stays claimed (Arriving) while the lock is dropped for the IddCx
    // calls below. A REMOVE or handle close in that window only marks it
    // cancelled; only this function frees an Arriving slot.
    MonitorSlot& s = m_Slots[idx];
    s.State = SlotState::Arriving;
    s.Cancelled = false;
    s.MonitorId = m_NextMonitorId++;
    s.ConnectorIndex = (UINT) idx;
    s.Owner = Owner;
    s.SerialSeed = In->SerialSeed;
    s.Monitor = nullptr;
    s.Context = nullptr;
    memcpy(s.Edid, edid, sizeof(edid));
    for (UINT i = 0; i < modeCount; i++) s.Modes[i] = modes[i];
    s.ModeCount = modeCount;

    UINT32 monitorId = s.MonitorId;
    UINT connectorIndex = s.ConnectorIndex;

    WdfWaitLockRelease(m_Lock);

    IDDCX_MONITOR_INFO mi = {};
    mi.Size = sizeof(mi);
    mi.MonitorType = DISPLAYCONFIG_OUTPUT_TECHNOLOGY_HDMI;
    mi.ConnectorIndex = connectorIndex;
    mi.MonitorDescription.Size = sizeof(mi.MonitorDescription);
    mi.MonitorDescription.Type = IDDCX_MONITOR_DESCRIPTION_TYPE_EDID;
    mi.MonitorDescription.DataSize = EXTEND_EDID_SIZE;
    mi.MonitorDescription.pData = edid;
    // A container id derived from the seed as well, so the same client device
    // groups under the same device entry every session.
    mi.MonitorContainerId = { In->SerialSeed, 0x1431, 0x463b, { 0xab, 0xdc, 0x0c, 0xb7, 0x68, 0xd6, 0xe6, 0x4a } };

    WDF_OBJECT_ATTRIBUTES attr;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attr, IndirectMonitorContextWrapper);
    attr.EvtCleanupCallback = [](WDFOBJECT Object)
    {
        auto* w = WdfObjectGet_IndirectMonitorContextWrapper(Object);
        if (w) w->Cleanup();
    };

    IDARG_IN_MONITORCREATE create = {};
    create.ObjectAttributes = &attr;
    create.pMonitorInfo = &mi;

    IDARG_OUT_MONITORCREATE createOut = {};
    NTSTATUS Status = IddCxMonitorCreate(m_Adapter, &create, &createOut);
    if (!NT_SUCCESS(Status))
    {
        WdfWaitLockAcquire(m_Lock, NULL);
        FreeSlotLocked(s);
        WdfWaitLockRelease(m_Lock);
        return Status;
    }

    // The mode list rides in the monitor context, which exists before arrival:
    // IddCx queries target modes during IddCxMonitorArrival.
    auto* w = WdfObjectGet_IndirectMonitorContextWrapper(createOut.MonitorObject);
    w->pContext = new IndirectMonitorContext(createOut.MonitorObject, monitorId, modes, modeCount, sddl);

    IDARG_OUT_MONITORARRIVAL arrival = {};
    Status = IddCxMonitorArrival(createOut.MonitorObject, &arrival);
    if (!NT_SUCCESS(Status))
    {
        DepartMonitor(createOut.MonitorObject, monitorId);
        WdfWaitLockAcquire(m_Lock, NULL);
        FreeSlotLocked(s);
        WdfWaitLockRelease(m_Lock);
        return Status;
    }

    WdfWaitLockAcquire(m_Lock, NULL);
    bool keep = !s.Cancelled && !FileClosed(Owner);
    s.State = keep ? SlotState::Active : SlotState::Departing;
    s.Monitor = createOut.MonitorObject;
    if (keep) s.Context = w->pContext;
    WdfWaitLockRelease(m_Lock);

    if (!keep)
    {
        // The owner closed its handle (or removed the id) during arrival. This
        // runs in our IOCTL, on IddCx's queue, like any departure must.
        ExtendLog("extend: monitor id=%u cancelled during arrival\n", monitorId);
        DepartSlot(s, createOut.MonitorObject, monitorId);
        return STATUS_CANCELLED;
    }

    Out->StructSize = sizeof(*Out);
    Out->MonitorId = monitorId;
    Out->OsAdapterLuidLow = arrival.OsAdapterLuid.LowPart;
    Out->OsAdapterLuidHigh = arrival.OsAdapterLuid.HighPart;
    Out->OsTargetId = arrival.OsTargetId;
    Out->ConnectorIndex = connectorIndex;

    ExtendLog("extend: added monitor id=%u connector=%u %ux%u\n", monitorId, connectorIndex, In->Width, In->Height);
    return STATUS_SUCCESS;
}

void IndirectDeviceContext::FreeSlotLocked(MonitorSlot& Slot)
{
    Slot = MonitorSlot();
}

NTSTATUS IndirectDeviceContext::DepartSlot(MonitorSlot& Slot, IDDCX_MONITOR Monitor, UINT32 MonitorId)
{
    // Only the thread that marked the slot Departing touches it until it is
    // free again, so the slot cannot be reused while the monitor departs. If
    // IddCx refuses the departure the monitor may still be there, so the slot
    // stays Departing for good: GET_INFO keeps counting it and its connector
    // and seed stay taken, rather than handing them to a new monitor.
    NTSTATUS status = DepartMonitor(Monitor, MonitorId);
    if (!NT_SUCCESS(status)) return status;
    WdfWaitLockAcquire(m_Lock, NULL);
    FreeSlotLocked(Slot);
    WdfWaitLockRelease(m_Lock);
    return status;
}

void IndirectDeviceContext::NoteControlQueue(WDFQUEUE Queue)
{
    WdfWaitLockAcquire(m_Lock, NULL);
    if (!m_ControlQueue) m_ControlQueue = Queue;
    WdfWaitLockRelease(m_Lock);
}

NTSTATUS IndirectDeviceContext::RemoveMonitor(WDFFILEOBJECT Owner, const EXTEND_REMOVE_MONITOR_IN* In)
{
    if (In->StructSize != sizeof(*In)) return STATUS_INVALID_PARAMETER;
    UINT32 id = In->MonitorId;

    MonitorSlot* departing = nullptr;
    IDDCX_MONITOR monitor = nullptr;
    NTSTATUS Status = STATUS_NOT_FOUND;
    WdfWaitLockAcquire(m_Lock, NULL);
    for (auto& s : m_Slots)
    {
        if (s.State == SlotState::Free || s.State == SlotState::Departing || s.MonitorId != id) continue;
        if (s.Owner != Owner) Status = STATUS_ACCESS_DENIED;
        else if (s.State == SlotState::Arriving) { s.Cancelled = true; Status = STATUS_SUCCESS; }
        else
        {
            s.State = SlotState::Departing;
            departing = &s;
            monitor = s.Monitor;
            Status = STATUS_SUCCESS;
        }
        break;
    }
    WdfWaitLockRelease(m_Lock);

    // This IOCTL runs on IddCx's sequential queue, so the departure cannot
    // overlap a swap-chain assignment, which arrives on the same queue.
    if (departing) Status = DepartSlot(*departing, monitor, id);
    return Status;
}

void IndirectDeviceContext::RemoveMonitorsForFile(WDFFILEOBJECT Owner)
{
    MonitorSlot* slots[EXTEND_MAX_MONITORS] = {};
    IDDCX_MONITOR monitors[EXTEND_MAX_MONITORS] = {};
    UINT32 ids[EXTEND_MAX_MONITORS] = {};
    UINT n = 0;

    WdfWaitLockAcquire(m_Lock, NULL);
    auto* file = WdfObjectGet_ExtendFileContext(Owner);
    if (file) file->Closed = true;
    for (auto& s : m_Slots)
    {
        if (s.Owner != Owner) continue;
        // An ADD still arriving departs its own monitor when it sees this.
        if (s.State == SlotState::Arriving) { s.Cancelled = true; continue; }
        if (s.State != SlotState::Active) continue;
        s.State = SlotState::Departing;
        slots[n] = &s;
        monitors[n] = s.Monitor;
        ids[n++] = s.MonitorId;
    }
    WDFQUEUE queue = m_ControlQueue;
    WdfWaitLockRelease(m_Lock);
    if (n == 0) return;

    // Cleanup does not arrive on IddCx's I/O-control queue, the sequential
    // queue that delivers our IOCTLs and also the kernel's messages, among
    // them "here is a new swap chain" (IddMonitor::HandleNewSwapChain, which
    // calls our assign callback). Departing a monitor while such a message is
    // being handled races IddCx's own handling of that monitor and crashes the
    // host, and Windows assigns a monitor its first swap chain shortly after it
    // arrives, just when a client that exits right after ADD closes its
    // handle. So hold that queue stopped while departing: once
    // WdfIoQueueStopSynchronously returns, the request it was handling is
    // complete and the rest wait, as they do while REMOVE departs from within
    // the queue. Departure itself needs nothing from the queue (REMOVE and
    // ADD depart while holding it). m_DepartLock keeps two closing handles
    // from restarting the queue under each other.
    WdfWaitLockAcquire(m_DepartLock, NULL);
    bool stopped = false;
    if (queue && WDF_IO_QUEUE_READY(WdfIoQueueGetState(queue, nullptr, nullptr)))
    {
        WdfIoQueueStopSynchronously(queue);
        stopped = true;
    }
    ExtendLog("extend: handle closed, departing %u monitor(s), control queue %s\n", n, stopped ? "stopped" : "not running");
    for (UINT i = 0; i < n; i++) DepartSlot(*slots[i], monitors[i], ids[i]);
    if (stopped) WdfIoQueueStart(queue);
    WdfWaitLockRelease(m_DepartLock);
}

NTSTATUS IndirectDeviceContext::QueryMonitor(WDFFILEOBJECT Owner, const EXTEND_QUERY_MONITOR_IN* In,
    EXTEND_QUERY_MONITOR_OUT* Out)
{
    if (In->StructSize != sizeof(*In)) return STATUS_INVALID_PARAMETER;
    UINT32 id = In->MonitorId; // read before Out is written; the buffers may alias

    WdfWaitLockAcquire(m_Lock, NULL);
    MonitorSlot* s = nullptr;
    for (auto& slot : m_Slots)
        if (slot.State == SlotState::Active && slot.MonitorId == id) { s = &slot; break; }

    if (!s) { WdfWaitLockRelease(m_Lock); return STATUS_NOT_FOUND; }
    if (s->Owner != Owner) { WdfWaitLockRelease(m_Lock); return STATUS_ACCESS_DENIED; }

    memset(Out, 0, sizeof(*Out));
    Out->StructSize = sizeof(*Out);
    Out->MonitorId = id;
    Out->Flags = EXTEND_MONITOR_FLAG_PRESENT;

    MonitorSurface& surf = s->Context->Surface();
    EnterCriticalSection(&surf.Lock);
    Out->SurfaceGeneration = surf.Generation;
    if (surf.Published)
    {
        Out->Flags |= EXTEND_MONITOR_FLAG_SURFACE;
        Out->RenderLuidLow = surf.RenderLuid.LowPart;
        Out->RenderLuidHigh = surf.RenderLuid.HighPart;
        Out->Width = surf.Width;
        Out->Height = surf.Height;
        Out->DxgiFormat = surf.Format;
        wcscpy_s(Out->TextureName, surf.Name);
    }
    if (surf.HadFrame) Out->Flags |= EXTEND_MONITOR_FLAG_FRAME;
    Out->FrameCounterLow = (UINT32)(surf.FrameCounter & 0xFFFFFFFFull);
    Out->FrameCounterHigh = (UINT32)(surf.FrameCounter >> 32);
    LeaveCriticalSection(&surf.Lock);

    WdfWaitLockRelease(m_Lock);
    return STATUS_SUCCESS;
}

bool IndirectDeviceContext::CopyModesForEdid(const void* EdidData, UINT EdidSize, EXTEND_MODE* Out, UINT* Count)
{
    if (EdidSize != EXTEND_EDID_SIZE || !EdidData) return false;

    bool found = false;
    WdfWaitLockAcquire(m_Lock, NULL);
    for (auto& s : m_Slots)
    {
        if (s.State != SlotState::Free && memcmp(s.Edid, EdidData, EXTEND_EDID_SIZE) == 0)
        {
            for (UINT i = 0; i < s.ModeCount; i++) Out[i] = s.Modes[i];
            *Count = s.ModeCount;
            found = true;
            break;
        }
    }
    WdfWaitLockRelease(m_Lock);
    return found;
}

#pragma endregion

#pragma region DDI Callbacks

_Use_decl_annotations_
VOID ExtendDeviceIoControl(WDFDEVICE Device, WDFREQUEST Request, size_t OutputBufferLength,
    size_t InputBufferLength, ULONG IoControlCode)
{
    auto* wrapper = WdfObjectGet_IndirectDeviceContextWrapper(Device);
    IndirectDeviceContext* ctx = wrapper ? wrapper->pContext : nullptr;
    NTSTATUS status = STATUS_INVALID_DEVICE_REQUEST;
    ULONG_PTR info = 0;

    WDFFILEOBJECT file = WdfRequestGetFileObject(Request);
    if (!ctx || !file)
    {
        WdfRequestComplete(Request, status);
        return;
    }
    ctx->NoteControlQueue(WdfRequestGetIoQueue(Request));

    switch (IoControlCode)
    {
    case IOCTL_EXTEND_GET_INFO:
    {
        EXTEND_INFO* out = nullptr;
        if (InputBufferLength != 0) { status = STATUS_INVALID_PARAMETER; break; }
        if (OutputBufferLength < sizeof(EXTEND_INFO)) { status = STATUS_BUFFER_TOO_SMALL; break; }
        status = WdfRequestRetrieveOutputBuffer(Request, sizeof(EXTEND_INFO), (PVOID*) &out, nullptr);
        if (!NT_SUCCESS(status)) break;
        status = ctx->GetInfo(out);
        if (NT_SUCCESS(status)) info = sizeof(EXTEND_INFO);
        break;
    }
    case IOCTL_EXTEND_ADD_MONITOR:
    {
        EXTEND_ADD_MONITOR_IN* in = nullptr;
        EXTEND_ADD_MONITOR_OUT* out = nullptr;
        if (InputBufferLength != sizeof(*in)) { status = STATUS_INVALID_PARAMETER; break; }
        if (OutputBufferLength < sizeof(*out)) { status = STATUS_BUFFER_TOO_SMALL; break; }
        status = WdfRequestRetrieveInputBuffer(Request, sizeof(*in), (PVOID*) &in, nullptr);
        if (!NT_SUCCESS(status)) break;
        status = WdfRequestRetrieveOutputBuffer(Request, sizeof(*out), (PVOID*) &out, nullptr);
        if (!NT_SUCCESS(status)) break;

        // The monitor's texture is scoped to the caller's logon SID, read from
        // its token at identification level (INF UmdfImpersonationLevel). A
        // handle whose QoS hides the token was already refused at open; this
        // fails closed too.
        ClientToken client;
        status = WdfRequestImpersonate(Request, SecurityIdentification, ExtendReadClientToken, &client);
        if (!NT_SUCCESS(status) || !NT_SUCCESS(client.Status))
        {
            ExtendLog("extend: ADD impersonation 0x%08x/0x%08x\n", status, client.Status);
            status = STATUS_BAD_IMPERSONATION_LEVEL;
            break;
        }

        // Copy the input: under METHOD_BUFFERED the output may share its buffer.
        EXTEND_ADD_MONITOR_IN request = *in;
        status = ctx->AddMonitor(file, client.Sid, &request, out);
        if (NT_SUCCESS(status)) info = sizeof(*out);
        break;
    }
    case IOCTL_EXTEND_REMOVE_MONITOR:
    {
        EXTEND_REMOVE_MONITOR_IN* in = nullptr;
        if (InputBufferLength != sizeof(*in)) { status = STATUS_INVALID_PARAMETER; break; }
        status = WdfRequestRetrieveInputBuffer(Request, sizeof(*in), (PVOID*) &in, nullptr);
        if (!NT_SUCCESS(status)) break;
        status = ctx->RemoveMonitor(file, in);
        break;
    }
    case IOCTL_EXTEND_QUERY_MONITOR:
    {
        EXTEND_QUERY_MONITOR_IN* in = nullptr;
        EXTEND_QUERY_MONITOR_OUT* out = nullptr;
        if (InputBufferLength != sizeof(*in)) { status = STATUS_INVALID_PARAMETER; break; }
        if (OutputBufferLength < sizeof(*out)) { status = STATUS_BUFFER_TOO_SMALL; break; }
        status = WdfRequestRetrieveInputBuffer(Request, sizeof(*in), (PVOID*) &in, nullptr);
        if (!NT_SUCCESS(status)) break;
        status = WdfRequestRetrieveOutputBuffer(Request, sizeof(*out), (PVOID*) &out, nullptr);
        if (!NT_SUCCESS(status)) break;
        status = ctx->QueryMonitor(file, in, out);
        if (NT_SUCCESS(status)) info = sizeof(*out);
        break;
    }
    default:
        break;
    }

    WdfRequestCompleteWithInformation(Request, status, info);
}

_Use_decl_annotations_
NTSTATUS ExtendAdapterInitFinished(IDDCX_ADAPTER AdapterObject, const IDARG_IN_ADAPTER_INIT_FINISHED* pInArgs)
{
    UNREFERENCED_PARAMETER(AdapterObject);
    UNREFERENCED_PARAMETER(pInArgs);
    // Unlike the sample, create NO monitors here. They arrive on client request.
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS ExtendAdapterCommitModes(IDDCX_ADAPTER AdapterObject, const IDARG_IN_COMMITMODES* pInArgs)
{
    UNREFERENCED_PARAMETER(AdapterObject);
    UNREFERENCED_PARAMETER(pInArgs);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS ExtendParseMonitorDescription(const IDARG_IN_PARSEMONITORDESCRIPTION* pInArgs, IDARG_OUT_PARSEMONITORDESCRIPTION* pOutArgs)
{
    IndirectDeviceContext* ctx = g_pDeviceContext;
    if (!ctx) return STATUS_INVALID_PARAMETER;

    EXTEND_MODE modes[EXTEND_MAX_MODES + 1];
    UINT count = 0;
    if (!ctx->CopyModesForEdid(pInArgs->MonitorDescription.pData, pInArgs->MonitorDescription.DataSize, modes, &count))
    {
        return STATUS_INVALID_PARAMETER;
    }

    pOutArgs->MonitorModeBufferOutputCount = count;
    if (pInArgs->MonitorModeBufferInputCount < count)
    {
        return (pInArgs->MonitorModeBufferInputCount > 0) ? STATUS_BUFFER_TOO_SMALL : STATUS_SUCCESS;
    }

    for (UINT k = 0; k < count; k++)
    {
        pInArgs->pMonitorModes[k] = CreateIddCxMonitorMode(
            modes[k].Width, modes[k].Height, modes[k].RefreshHz, IDDCX_MONITOR_MODE_ORIGIN_MONITORDESCRIPTOR);
    }
    pOutArgs->PreferredMonitorModeIdx = 0; // the requested mode
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS ExtendMonitorGetDefaultModes(IDDCX_MONITOR MonitorObject, const IDARG_IN_GETDEFAULTDESCRIPTIONMODES* pInArgs, IDARG_OUT_GETDEFAULTDESCRIPTIONMODES* pOutArgs)
{
    UNREFERENCED_PARAMETER(MonitorObject);
    // We always supply an EDID, so this is normally not used. Offer one safe mode.
    if (pInArgs->DefaultMonitorModeBufferInputCount == 0)
    {
        pOutArgs->DefaultMonitorModeBufferOutputCount = 1;
    }
    else
    {
        pInArgs->pDefaultMonitorModes[0] = CreateIddCxMonitorMode(1024, 768, 60, IDDCX_MONITOR_MODE_ORIGIN_DRIVER);
        pOutArgs->DefaultMonitorModeBufferOutputCount = 1;
        pOutArgs->PreferredMonitorModeIdx = 0;
    }
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS ExtendMonitorQueryModes(IDDCX_MONITOR MonitorObject, const IDARG_IN_QUERYTARGETMODES* pInArgs, IDARG_OUT_QUERYTARGETMODES* pOutArgs)
{
    // Offer exactly the monitor's own modes: the client's requested mode and its
    // fallbacks, so any valid size activates as requested. They come from the
    // monitor context, which is set before arrival, when IddCx first asks.
    auto* w = WdfObjectGet_IndirectMonitorContextWrapper(MonitorObject);
    if (!w || !w->pContext) return STATUS_INVALID_PARAMETER;

    const EXTEND_MODE* modes = w->pContext->Modes();
    UINT count = w->pContext->ModeCount();
    pOutArgs->TargetModeBufferOutputCount = count;
    if (pInArgs->TargetModeBufferInputCount >= count)
    {
        for (UINT i = 0; i < count; i++)
        {
            pInArgs->pTargetModes[i] = CreateIddCxTargetMode(modes[i].Width, modes[i].Height, modes[i].RefreshHz);
        }
    }
    return STATUS_SUCCESS;
}

// The monitor callbacks below check the context first. Departures are
// serialised with IddCx's queue, so IddCx should never call them for a monitor
// whose object (and our context with it) is gone; if it ever does, they touch
// nothing of ours.

_Use_decl_annotations_
NTSTATUS ExtendMonitorAssignSwapChain(IDDCX_MONITOR MonitorObject, const IDARG_IN_SETSWAPCHAIN* pInArgs)
{
    auto* w = WdfObjectGet_IndirectMonitorContextWrapper(MonitorObject);
    if (!w || !w->pContext)
    {
        // The documented way to refuse a swap chain: IddCx abandons it rather
        // than reporting a driver failure.
        ExtendLog("extend: swap chain for a monitor that is gone, abandoned\n");
        return STATUS_GRAPHICS_INDIRECT_DISPLAY_ABANDON_SWAPCHAIN;
    }
    w->pContext->AssignSwapChain(pInArgs->hSwapChain, pInArgs->RenderAdapterLuid, pInArgs->hNextSurfaceAvailable);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS ExtendMonitorUnassignSwapChain(IDDCX_MONITOR MonitorObject)
{
    auto* w = WdfObjectGet_IndirectMonitorContextWrapper(MonitorObject);
    if (w && w->pContext) w->pContext->UnassignSwapChain();
    return STATUS_SUCCESS;
}

#pragma endregion
