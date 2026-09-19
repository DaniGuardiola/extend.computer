import { useCallback, useEffect, useRef, useState } from "react";
import { Plus, Settings, X, LogIn, Monitor, Unlink } from "lucide-react";
import {
  api,
  native,
  type Device,
  type Snapshot,
  type Permissions,
} from "./bridge";
import {
  PermissionContinuation,
  permissionsReady,
} from "./PermissionContinuation";
import { usePermissions } from "./usePermissions";
import { useAppearance } from "./Appearance";
import { DeviceRow } from "./DeviceRow";
import { PairDialog } from "./PairDialog";
import { PermissionsDialog, type PermissionAction } from "./PermissionsDialog";
import { SettingsScreen } from "./SettingsScreen";
import { PairingApprovalDialog } from "./PairingApprovalDialog";
import { ExplainedButton } from "./ExplainedButton";
export function App() {
  const appearance = useAppearance();
  const [data, setData] = useState<Snapshot | null>(null);
  const [error, setError] = useState("");
  const [pair, setPair] = useState(false);
  const pairingPeerCount = useRef(0);
  useEffect(() => {
    if (
      pair &&
      data &&
      data.peers.length > pairingPeerCount.current &&
      !data.session
    ) {
      setPair(false);
      void api.closePairing();
    }
  }, [pair, data]);
  const [settings, setSettings] = useState(false);
  const [permissionAction, setPermissionAction] =
    useState<PermissionAction | null>(null);
  const continuation = useRef(new PermissionContinuation());
  const lastShare = useRef<{ peer: string; device: Device } | null>(null);
  const waitForReceiving = () => {
    continuation.current.waitFor({ kind: "receive" });
    setPermissionAction("receive");
  };
  const [busy, setBusy] = useState(false);
  const [receivingBusy, setReceivingBusy] = useState(false);
  const receivingTask = useRef<Promise<void> | null>(null);
  const updatePermissions = useCallback((value: Permissions) => {
    setData((current) =>
      current ? { ...current, permissions: value } : current,
    );
  }, []);
  const permissionFlow = usePermissions(
    settings || permissionAction !== null,
    updatePermissions,
  );

  const refresh = useCallback(async () => {
    if (native) setData(await api.snapshot());
  }, []);
  useEffect(() => {
    let done = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        if (native) {
          const next = await api.snapshot();
          if (!done) setData(next);
        }
      } catch (e) {
        if (!done) setError(String(e));
      }
      if (!done) timer = setTimeout(poll, 500);
    };
    void poll();
    return () => {
      done = true;
      clearTimeout(timer);
    };
  }, []);
  const act = async (action: () => Promise<void>) => {
    setBusy(true);
    setError("");
    try {
      await receivingTask.current;
      await action();
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const toggleReceiving = () => {
    if (receivingTask.current || busy || !data) return;
    if (
      !data.receiving &&
      (!permissionFlow.checked ||
        !data.permissions.available ||
        !data.permissions.post ||
        !data.permissions.wifi)
    ) {
      setError("");
      waitForReceiving();
      return;
    }
    setReceivingBusy(true);
    setError("");
    const task = (async () => {
      try {
        if (data.receiving) {
          await api.stopReceiving();
        } else {
          const access = await permissionFlow.check();
          if (!access.available || !access.post || !access.wifi) {
            waitForReceiving();
            return;
          }
          await api.receive(false);
        }
        await refresh();
      } catch (e) {
        setError(String(e));
      } finally {
        setReceivingBusy(false);
      }
    })();
    receivingTask.current = task;
    void task.finally(() => {
      if (receivingTask.current === task) receivingTask.current = null;
    });
  };
  const connect = (peer: string, device: Device) => {
    lastShare.current = { peer, device };
    void act(async () => {
      const access = await permissionFlow.check();
      if (!permissionsReady("share", access)) {
        continuation.current.waitFor({ kind: "share", peer, device });
        setPermissionAction("share");
        return;
      }
      await api.connect(peer, device);
    });
  };
  const permissionRequest = data?.permission_request;
  useEffect(() => {
    if (!permissionRequest) return;
    if (permissionRequest === "receive") {
      continuation.current.waitFor({ kind: "receive" });
    } else if (lastShare.current) {
      continuation.current.waitFor({ kind: "share", ...lastShare.current });
    }
    setPermissionAction(permissionRequest);
  }, [permissionRequest]);
  useEffect(() => {
    if (data?.session?.phase === "connected") lastShare.current = null;
    if (
      !permissionAction ||
      !data ||
      !permissionFlow.checked ||
      busy ||
      receivingBusy ||
      data.session
    )
      return;
    const pending = continuation.current.takeReady(data.permissions);
    if (!pending) return;
    setPermissionAction(null);
    void act(async () => {
      const access = await permissionFlow.check();
      if (!permissionsReady(pending.kind, access)) {
        continuation.current.waitFor(pending);
        setPermissionAction(pending.kind);
        return;
      }
      await api.dismiss();
      if (pending.kind === "receive") {
        if (!data.receiving) await api.receive(false);
      } else {
        const peer = data.peers.find((peer) => peer.id === pending.peer);
        if (!peer) throw new Error("This device is no longer paired.");
        await api.connect(peer.id, peer);
      }
    });
  });
  const notificationId = data?.notification?.id;
  useEffect(() => {
    if (notificationId === undefined) return;
    let timer: ReturnType<typeof setTimeout>;
    const schedule = () => {
      clearTimeout(timer);
      if (document.visibilityState === "visible")
        timer = setTimeout(
          () => void api.dismissNotification(notificationId),
          8000,
        );
    };
    schedule();
    document.addEventListener("visibilitychange", schedule);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", schedule);
    };
  }, [notificationId]);
  const session = data?.session;
  return (
    <div className="flex h-screen flex-col px-8">
      <header
        data-tauri-drag-region="deep"
        className={`flex h-14 shrink-0 select-none items-center justify-between border-b border-line ${native && navigator.platform.startsWith("Mac") ? "pl-19" : ""}`}
      >
        <div className="flex items-center gap-2.5 text-lg font-medium tracking-[-.5px]">
          <img
            src="/extend-computer.svg"
            alt=""
            className="brand-logo size-7"
          />
          extend.computer
          {data?.development && (
            <span className="text-[10px] font-normal tracking-normal text-muted">
              DEV
            </span>
          )}
        </div>
        <button
          className="button quiet !px-2 !text-muted"
          disabled
          title="Accounts are not available yet"
        >
          <LogIn size={14} />
          Log in
        </button>
      </header>
      <main className="-mx-8 min-h-0 flex-1 overflow-y-auto overscroll-contain px-8 py-7">
        {settings && data ? (
          <SettingsScreen
            theme={appearance.theme}
            onThemeChange={appearance.setTheme}
            snapshot={data}
            checked={permissionFlow.checked}
            error={permissionFlow.error}
            openPermission={permissionFlow.open}
            onBack={() => setSettings(false)}
          />
        ) : (
          <>
            <div className="mb-6 flex items-center justify-between">
              <h1 className="text-base font-medium">Devices</h1>
              <button
                className="button quiet"
                disabled={!data || busy || !!session}
                onClick={() =>
                  void act(async () => {
                    await api.receive(true);
                    pairingPeerCount.current = data?.peers.length ?? 0;
                    setPair(true);
                  })
                }
              >
                <Plus size={15} />
                Pair device
              </button>
            </div>
            {!native && (
              <p className="mb-4 text-sm text-muted">
                Desktop preview. Open extend.computer to connect devices.
              </p>
            )}
            {(error || data?.error) && (
              <div
                role="alert"
                className="error mb-4 flex items-center justify-between gap-3"
              >
                <span className="min-w-0 flex-1 break-words leading-5">
                  {error || data?.error}
                </span>
                <button
                  className="flex size-8 shrink-0 items-center justify-center rounded-full hover:bg-danger/10 focus-visible:outline-danger"
                  aria-label="Dismiss error"
                  onClick={() => {
                    setError("");
                    void api.dismiss();
                  }}
                >
                  <X size={14} />
                </button>
              </div>
            )}
            {data && data.peers.length + data.removed_peers.length > 0 ? (
              <ul className="space-y-3">
                {[...data.peers, ...data.removed_peers]
                  .sort((a, b) => {
                    const rank = (p: typeof a) =>
                      "reason" in p
                        ? 4
                        : p.availability === "online" ||
                            (session?.phase === "connected" &&
                              session.peer === p.id)
                          ? 0
                          : p.availability === "receiving_off"
                            ? 1
                            : p.availability === "checking"
                              ? 2
                              : 3;
                    return (
                      rank(a) - rank(b) ||
                      a.name.localeCompare(b.name) ||
                      a.id.localeCompare(b.id)
                    );
                  })
                  .map((peer) =>
                    "reason" in peer ? (
                      <li
                        key={peer.id}
                        className="flex items-center justify-between gap-4 rounded-xl bg-placeholder p-4 text-muted"
                      >
                        <div>
                          <p className="flex items-center gap-2 text-sm font-medium">
                            <Unlink size={14} aria-hidden="true" />
                            {peer.name}
                          </p>
                          <p className="mt-1 text-xs leading-5 text-muted">
                            {peer.reason}
                          </p>
                        </div>
                        <button
                          className="button quiet"
                          disabled={busy || !!session}
                          onClick={() =>
                            void act(() => api.dismissRemoved(peer.id))
                          }
                        >
                          Dismiss
                        </button>
                      </li>
                    ) : (
                      <DeviceRow
                        key={peer.id}
                        peer={peer}
                        disabled={!!session || busy}
                        connect={connect}
                        session={session?.peer === peer.id ? session : null}
                        pending={busy}
                        disconnect={() => void act(api.disconnect)}
                        changeEdge={(id, device) =>
                          void act(() => api.update(id, device))
                        }
                        refresh={refresh}
                      />
                    ),
                  )}
              </ul>
            ) : (
              <div className="flex min-h-60 flex-col items-center justify-center gap-4 rounded-xl border border-dashed border-line">
                <Monitor size={28} strokeWidth={1.5} className="text-muted" />
                <p className="text-sm text-muted">No paired devices</p>
                <button
                  className="button quiet"
                  disabled={!data || !!session}
                  onClick={() =>
                    void act(async () => {
                      await api.receive(true);
                      pairingPeerCount.current = data?.peers.length ?? 0;
                      setPair(true);
                    })
                  }
                >
                  Pair a device
                </button>
              </div>
            )}
          </>
        )}
      </main>
      {data?.notification && (
        <div
          role="status"
          className="mb-3 flex items-center justify-between gap-3 rounded-xl border border-line bg-line/30 p-3 text-sm"
        >
          <span>{data.notification.message}</span>
          <button
            className="icon-button"
            aria-label="Dismiss notification"
            onClick={() => void api.dismissNotification(data.notification!.id)}
          >
            <X size={14} />
          </button>
        </div>
      )}
      <footer className="flex h-14 shrink-0 items-center gap-4 border-t border-line text-xs text-muted">
        <button
          className={`button -ms-3.5 ${settings ? "bg-line text-ink" : "quiet"}`}
          aria-pressed={settings}
          disabled={!data}
          onClick={() => setSettings((open) => !open)}
        >
          <Settings size={14} aria-hidden="true" />
          Settings
        </button>
        <span className="h-4 border-l border-line" aria-hidden="true" />
        <ExplainedButton
          type="button"
          role="switch"
          aria-label="Allow paired devices to control this device"
          aria-checked={data?.receiving ?? false}
          aria-disabled={!data || busy || receivingBusy}
          aria-busy={receivingBusy}
          disabled={!data || busy}
          explanation="Allow paired devices to connect to and control this device."
          className="button quiet"
          onClick={toggleReceiving}
        >
          <span
            aria-hidden="true"
            className={`size-1.5 shrink-0 rounded-full ${data?.receiving ? "bg-emerald-500" : "bg-muted/50"}`}
          />
          <span className="min-w-20 text-start">Allow connections</span>
          <span
            aria-hidden="true"
            className={`flex h-4 w-7 shrink-0 items-center rounded-full p-0.5 motion-safe:transition-colors ${data?.receiving ? "bg-emerald-600" : "bg-muted/35"}`}
          >
            <span
              className={`size-3 rounded-full bg-white shadow-sm motion-safe:transition-transform ${data?.receiving ? "translate-x-3" : "translate-x-0"}`}
            />
          </span>
        </ExplainedButton>
      </footer>
      {pair && data && !data.approval && (
        <PairDialog
          snapshot={data}
          refresh={refresh}
          onClose={() => {
            setPair(false);
            void api.closePairing();
          }}
        />
      )}
      {permissionAction && data && (
        <PermissionsDialog
          action={permissionAction}
          snapshot={data}
          checked={permissionFlow.checked}
          error={permissionFlow.error}
          openPermission={permissionFlow.open}
          onClose={() => {
            continuation.current.cancel();
            lastShare.current = null;
            setPermissionAction(null);
            void api.dismiss();
          }}
        />
      )}
      {data?.approval && (
        <PairingApprovalDialog
          key={data.approval.id}
          request={data.approval}
          onDone={refresh}
        />
      )}
    </div>
  );
}
