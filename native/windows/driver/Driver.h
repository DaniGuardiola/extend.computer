/*++

Copyright (c) Microsoft Corporation

    Portions of this file are derived from Microsoft's IddSampleDriver
    (Windows-driver-samples, video/IndirectDisplay), licensed under the
    Microsoft Public License (MS-PL). See native/windows/driver/README.md and the
    LICENSE-MS-PL notice retained there. Modifications for extend.computer add a
    client-controlled monitor lifecycle, a custom IOCTL control contract, and a
    keyed-mutex shared-texture frame handoff.

Abstract:

    Local type definitions for the extend.computer indirect display driver.

Environment:

    Windows User-Mode Driver Framework 2

--*/

#pragma once

#define NOMINMAX
// windows.h defines a subset of the NTSTATUS codes that ntstatus.h (pulled in by
// the WDF headers) defines again. WIN32_NO_STATUS suppresses windows.h's copies
// so the build is free of C4005 redefinition warnings.
#define WIN32_NO_STATUS
#include <windows.h>
#undef WIN32_NO_STATUS
#include <ntstatus.h>
#include <bugcodes.h>
#include <wudfwdm.h>
#include <wdf.h>
#include <iddcx.h>

#include <dxgi1_5.h>
#include <d3d11_2.h>
#include <avrt.h>
#include <wrl.h>

#include <atomic>
#include <memory>
#include <vector>

#include "Protocol.h"

#define EXTEND_EDID_SIZE 128
#define EXTEND_SDDL_LEN  256 // WCHARs; holds the texture SDDL with one user/logon SID

namespace Microsoft
{
    namespace WRL
    {
        namespace Wrappers
        {
            typedef HandleT<HandleTraits::HANDLENullTraits> Thread;
        }
    }
}

namespace ExtendComputer
{
    // Published per-monitor surface state, written by the swap-chain processor
    // thread and read by IOCTL_EXTEND_QUERY_MONITOR. Guarded by its own lock.
    struct MonitorSurface
    {
        CRITICAL_SECTION Lock;
        bool   Published = false;     // shared texture exists, name valid
        bool   HadFrame = false;      // at least one frame published
        UINT32 Generation = 0;        // bumps with every new shared texture
        LUID   RenderLuid = {};
        UINT   Width = 0;
        UINT   Height = 0;
        UINT   Format = 0;            // DXGI_FORMAT
        UINT64 FrameCounter = 0;      // monotonic across generations
        wchar_t Name[EXTEND_TEXTURE_NAME_LEN] = {}; // full "Global\..." name

        MonitorSurface() { InitializeCriticalSection(&Lock); }
        ~MonitorSurface() { DeleteCriticalSection(&Lock); }
        MonitorSurface(const MonitorSurface&) = delete;
        MonitorSurface& operator=(const MonitorSurface&) = delete;
    };

    struct Direct3DDevice
    {
        Direct3DDevice(LUID AdapterLuid);
        Direct3DDevice();
        HRESULT Init();

        LUID AdapterLuid;
        Microsoft::WRL::ComPtr<IDXGIFactory5> DxgiFactory;
        Microsoft::WRL::ComPtr<IDXGIAdapter1> Adapter;
        Microsoft::WRL::ComPtr<ID3D11Device> Device;
        Microsoft::WRL::ComPtr<ID3D11DeviceContext> DeviceContext;
    };

    class IndirectMonitorContext;

    // Consumes buffers from an indirect-display swap chain and copies each frame
    // into a per-monitor keyed-mutex shared texture for the client to read. When
    // the client holds the texture, the newest frame waits in a driver-private
    // copy and is published as soon as the client releases it.
    //
    // The driver owns the swap chain once it accepts it (as the IddCx docs and
    // the sample have it): the processing thread services it until it is told
    // to stop (EvtIddCxMonitorUnassignSwapChain, which IddCx also calls from
    // IddCxMonitorDeparture) or the swap chain fails, and deletes it
    // (WdfObjectDelete) as it exits. Every assigned swap chain is therefore
    // serviced until IddCx takes it back, so its "driver did not release a
    // frame" watchdog never waits on a live, unserviced one. The one exception
    // is OrphanSwapChain: when the monitor object is deleted without a
    // departure (device teardown), WDF deletes the swap chain with it, so the
    // thread must not.
    class SwapChainProcessor
    {
    public:
        SwapChainProcessor(IDDCX_SWAPCHAIN hSwapChain, std::shared_ptr<Direct3DDevice> Device,
            HANDLE NewFrameEvent, IndirectMonitorContext* Monitor);
        // Stops the thread and waits for it. The thread checks for the stop on
        // every frame, so this returns promptly even while frames keep coming.
        ~SwapChainProcessor();

        // The swap chain is going away with its monitor object, so the thread
        // must not delete it. Call before destroying the processor.
        void OrphanSwapChain() { m_SwapChainOrphaned = true; }

        // Whether the processing thread has not exited yet.
        bool Running() const { return m_hThread.Get() && WaitForSingleObject(m_hThread.Get(), 0) == WAIT_TIMEOUT; }

    private:
        static DWORD CALLBACK RunThread(LPVOID Argument);

        void Run();
        void RunCore();
        bool EnsureSharedTexture(const D3D11_TEXTURE2D_DESC& Source);
        void ReleaseSharedTexture();
        HRESULT AcquireForWrite();
        void Publish(ID3D11Texture2D* Source);
        void PublishFrame(ID3D11Texture2D* Source);
        void PublishPending();
        bool RecoverFromMutexFailure(HRESULT hr);

        IDDCX_SWAPCHAIN m_hSwapChain;
        std::atomic<bool> m_SwapChainOrphaned { false };
        std::shared_ptr<Direct3DDevice> m_Device;
        HANDLE m_hAvailableBufferEvent;
        Microsoft::WRL::Wrappers::Thread m_hThread;
        Microsoft::WRL::Wrappers::Event m_hTerminateEvent;

        IndirectMonitorContext* m_Monitor; // owns this processor, outlives it
        Microsoft::WRL::ComPtr<ID3D11Texture2D> m_SharedTexture;
        Microsoft::WRL::ComPtr<IDXGIKeyedMutex> m_KeyedMutex;
        HANDLE m_SharedHandle = nullptr;
        UINT32 m_Generation = 0;           // surface generation of m_SharedTexture
        Microsoft::WRL::ComPtr<ID3D11Texture2D> m_LatestTexture; // driver-private newest frame
        D3D11_TEXTURE2D_DESC m_Desc = {};  // shape of both textures
        bool m_Pending = false;            // m_LatestTexture holds an unpublished frame
        bool m_DeviceLost = false;
        ULONGLONG m_RetryAfter = 0;        // backoff after a failed texture creation
    };

    class IndirectMonitorContext
    {
    public:
        IndirectMonitorContext(IDDCX_MONITOR Monitor, UINT32 MonitorId, const EXTEND_MODE* Modes, UINT ModeCount,
            const wchar_t* TextureSddl);
        virtual ~IndirectMonitorContext();

        void AssignSwapChain(IDDCX_SWAPCHAIN SwapChain, LUID RenderAdapter, HANDLE NewFrameEvent);
        void UnassignSwapChain();

        MonitorSurface& Surface() { return m_Surface; }
        UINT32 MonitorId() const { return m_MonitorId; }
        UINT32 Nonce() const { return m_Nonce; }
        const wchar_t* TextureSddl() const { return m_TextureSddl; }

        // The monitor's mode list, [0] preferred. Immutable after construction,
        // and set before IddCxMonitorArrival so QueryTargetModes can read it
        // during arrival.
        const EXTEND_MODE* Modes() const { return m_Modes; }
        UINT ModeCount() const { return m_ModeCount; }

    private:
        IDDCX_MONITOR m_Monitor;
        UINT32 m_MonitorId;
        UINT32 m_Nonce;
        EXTEND_MODE m_Modes[EXTEND_MAX_MODES + 1] = {};
        UINT m_ModeCount = 0;
        wchar_t m_TextureSddl[EXTEND_SDDL_LEN] = {};
        MonitorSurface m_Surface;

        // Guards m_ProcessingThread and m_Retired. IddCx's assign/unassign
        // callbacks and the context's cleanup take it; the processing threads
        // never do, so joining them under the lock is safe.
        CRITICAL_SECTION m_ProcessingLock;
        std::unique_ptr<SwapChainProcessor> m_ProcessingThread;
        // Processors whose swap chain failed and that are still deleting it
        // when the next one arrives; see AssignSwapChain.
        std::vector<std::unique_ptr<SwapChainProcessor>> m_Retired;
    };

    enum class SlotState
    {
        Free,
        Arriving,  // claimed by an in-flight ADD; the monitor is being created
        Active,
        Departing, // removed; its departure is running. Keeps the connector
                   // and the seed taken until the monitor is gone.
    };

    // One slot in the adapter's monitor table. The adapter starts with every slot
    // free; clients fill them with IOCTL_EXTEND_ADD_MONITOR.
    struct MonitorSlot
    {
        SlotState State = SlotState::Free;
        bool Cancelled = false;        // owner closed or removed it during arrival
        UINT32 MonitorId = 0;          // opaque, monotonically assigned
        UINT ConnectorIndex = 0;       // this slot's index (0..EXTEND_MAX_MONITORS-1)
        WDFFILEOBJECT Owner = nullptr; // file handle that created it; cleanup departs it
                                       // (valid while the slot is used: cleanup frees or
                                       // cancels every slot before the handle can close)
        UINT32 SerialSeed = 0;
        IDDCX_MONITOR Monitor = nullptr;
        IndirectMonitorContext* Context = nullptr;
        BYTE Edid[EXTEND_EDID_SIZE] = {};
        EXTEND_MODE Modes[EXTEND_MAX_MODES + 1] = {}; // [0] = preferred, then fallbacks
        UINT ModeCount = 0;
    };

    class IndirectDeviceContext
    {
    public:
        IndirectDeviceContext(WDFDEVICE WdfDevice);
        virtual ~IndirectDeviceContext();

        void InitAdapter();

        // IOCTL handlers.
        NTSTATUS GetInfo(EXTEND_INFO* Out);
        NTSTATUS AddMonitor(WDFFILEOBJECT Owner, const wchar_t* OwnerSid, const EXTEND_ADD_MONITOR_IN* In,
            EXTEND_ADD_MONITOR_OUT* Out);
        NTSTATUS RemoveMonitor(WDFFILEOBJECT Owner, const EXTEND_REMOVE_MONITOR_IN* In);
        NTSTATUS QueryMonitor(WDFFILEOBJECT Owner, const EXTEND_QUERY_MONITOR_IN* In, EXTEND_QUERY_MONITOR_OUT* Out);

        // Departs every monitor owned by a file handle that is closing, and marks
        // the handle closed so an ADD still in flight on it creates nothing.
        void RemoveMonitorsForFile(WDFFILEOBJECT Owner);

        // Remembers the I/O-control queue our IOCTLs arrive on, which is IddCx's
        // own sequential queue. See RemoveMonitorsForFile.
        void NoteControlQueue(WDFQUEUE Queue);

        // Copies the mode list of the monitor whose EDID this is, under the lock.
        // IddCx identifies the monitor only by its description in
        // ParseMonitorDescription, which runs during arrival.
        bool CopyModesForEdid(const void* EdidData, UINT EdidSize, EXTEND_MODE* Out, UINT* Count);

        IDDCX_ADAPTER Adapter() const { return m_Adapter; }

    private:
        // Frees a slot. Caller holds m_Lock.
        void FreeSlotLocked(MonitorSlot& Slot);

        // Departs a Departing slot's monitor and then frees the slot, unless
        // IddCx refused the departure (returned). Called without m_Lock:
        // departure calls back into the driver.
        NTSTATUS DepartSlot(MonitorSlot& Slot, IDDCX_MONITOR Monitor, UINT32 MonitorId);

        WDFDEVICE m_WdfDevice;
        IDDCX_ADAPTER m_Adapter;
        WDFWAITLOCK m_Lock = nullptr;
        WDFWAITLOCK m_DepartLock = nullptr; // serialises departures made outside the queue
        WDFQUEUE m_ControlQueue = nullptr;  // guarded by m_Lock
        MonitorSlot m_Slots[EXTEND_MAX_MONITORS];
        UINT32 m_NextMonitorId = 1;
        UINT m_IddCxVersion = 0;
    };
}

// Synthesises a valid 128-byte EDID 1.4 block for a virtual monitor. Defined in Edid.cpp.
void ExtendBuildEdid(BYTE Out[EXTEND_EDID_SIZE], UINT Width, UINT Height, UINT RefreshHz,
    UINT PhysicalWidthMm, UINT PhysicalHeightMm, UINT32 Serial);
