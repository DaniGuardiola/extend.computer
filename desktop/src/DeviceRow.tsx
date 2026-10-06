import { useEffect, useState } from "react";
import { Tooltip, TooltipAnchor, TooltipProvider } from "@ariakit/react";
import {
  Monitor,
  LoaderCircle,
  Keyboard,
  Copy,
  MonitorUp,
  Square,
  Settings2,
  Unlink,
} from "lucide-react";
import { api, type Device, type Peer, type Snapshot } from "./bridge";
import { Select } from "./Select";
import { Dialog } from "./Dialog";
import { ExplainedButton } from "./ExplainedButton";
const availabilityStyle = {
  update_required: { label: "Update required", dot: "bg-amber-500" },
  online: { label: "Online", dot: "bg-emerald-500" },
  receiving_off: { label: "Not accepting connections", dot: "bg-amber-500" },
  offline: { label: "Offline", dot: "bg-muted/60" },
  checking: {
    label: "Checking…",
    dot: "bg-muted/60 motion-safe:animate-pulse",
  },
};

export function DeviceRow({
  peer,
  disabled,
  connect,
  session,
  pending,
  disconnect,
  changeEdge,
  refresh,
}: {
  peer: Peer;
  disabled: boolean;
  session: Snapshot["session"];
  pending: boolean;
  disconnect: () => void;
  changeEdge: (peer: string, device: Device) => void;
  connect: (peer: string, device: Device) => void;
  refresh: () => Promise<void>;
}) {
  const connected = session?.phase === "connected";
  const active = connected && session?.kind === "outgoing";
  const transitioning = !!session && !connected;
  const connectionLabel = connected
    ? session?.kind === "incoming"
      ? "Connected (incoming)"
      : "Connected"
    : session?.phase === "disconnecting"
      ? "Disconnecting…"
      : session?.kind === "unpair"
        ? "Unpairing…"
        : "Connecting…";
  const status =
    availabilityStyle[active ? "online" : peer.availability] ??
    availabilityStyle.checking;
  const [settings, setSettings] = useState(false);
  const [confirmUnpair, setConfirmUnpair] = useState(false);
  const [device, setDevice] = useState<Device>({
    name: peer.name,
    address: peer.address,
    edge: peer.edge,
  });
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    setDevice({ name: peer.name, address: peer.address, edge: peer.edge });
  }, [peer.name, peer.address, peer.edge]);
  return (
    <li
      className={`overflow-hidden rounded-xl border ${connected ? "device-connected" : "border-line bg-surface/50"}`}
    >
      <div className="flex items-start gap-3 px-5 pt-4 pb-3">
        <Monitor
          size={20}
          className={`shrink-0 ${connected ? "text-success" : "text-muted"}`}
        />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <p className="truncate text-sm font-medium">{peer.name}</p>
            <TooltipProvider timeout={400}>
              <TooltipAnchor
                render={
                  <span
                    tabIndex={0}
                    className="shrink-0 cursor-help rounded-md border border-line px-1.5 py-0.5 text-[10px] font-medium leading-3 text-muted"
                  />
                }
              >
                {peer.trust_source === "account" ? "Account" : "Paired"}
              </TooltipAnchor>
              <Tooltip className="explanation-tooltip">
                {peer.trust_source === "account"
                  ? "Signed in to the same account."
                  : "Paired directly with this device. No account required."}
              </Tooltip>
            </TooltipProvider>
          </div>
          {session ? (
            <div
              className={`mt-1 flex min-h-7 flex-wrap items-center gap-x-3 gap-y-2 text-xs ${connected ? "text-success" : "text-muted"}`}
            >
              <span
                className={`inline-flex items-center gap-1.5 rounded-full px-2 py-1 font-medium ${connected ? "bg-success/15" : "bg-line/50"}`}
              >
                {transitioning ? (
                  <LoaderCircle
                    size={12}
                    aria-hidden="true"
                    className="motion-safe:animate-spin"
                  />
                ) : (
                  <span
                    aria-hidden="true"
                    className="size-1.5 rounded-full bg-success"
                  />
                )}
                {connectionLabel}
              </span>
              <span className="inline-flex items-center gap-1.5">
                <Keyboard size={14} aria-hidden="true" />
                Keyboard &amp; mouse
              </span>
              {connected && session.route && (
                <span>
                  {session.route === "local" ? "Local network" : "Internet relay"}
                </span>
              )}
            </div>
          ) : (
            <p className="mt-1 flex min-h-7 items-center gap-1.5 text-xs text-muted">
              <span
                aria-hidden="true"
                className={`size-1.5 shrink-0 rounded-full ${status.dot}`}
              />
              {peer.availability === "receiving_off" ? (
                <TooltipProvider timeout={400}>
                  <TooltipAnchor
                    render={<span tabIndex={0} className="cursor-help" />}
                  >
                    {status.label}
                  </TooltipAnchor>
                  <Tooltip className="explanation-tooltip">
                    Open extend.computer on {peer.name} and turn on Allow
                    connections.
                  </Tooltip>
                </TooltipProvider>
              ) : (
                status.label
              )}
            </p>
          )}
        </div>
        <button
          className="icon-button -mt-1.5"
          aria-label={`Settings for ${peer.name}`}
          onClick={() => setSettings(true)}
        >
          <Settings2 size={16} />
        </button>
      </div>
      {!session && peer.availability === "update_required" && (
        <p className="px-5 pb-4 text-xs text-muted">
          Update extend.computer on both devices to reconnect.
        </p>
      )}
      {(session || peer.availability === "online") && (
        <div className="grid grid-cols-2 gap-1 px-3 pb-3 min-[720px]:grid-cols-4">
          <ExplainedButton
            className="device-action"
            aria-disabled="true"
            explanation="Use this device as an extra display. Coming later."
          >
            <MonitorUp size={18} />
            <span>Extend display</span>
          </ExplainedButton>
          <ExplainedButton
            className="device-action"
            aria-disabled="true"
            explanation="Show the same screen on both devices. Coming later."
          >
            <Copy size={18} />
            <span>Mirror display</span>
          </ExplainedButton>
          <ExplainedButton
            className={`device-action ${active ? "bg-line/40" : ""}`}
            disabled={
              session
                ? pending ||
                  session.phase === "disconnecting" ||
                  session.kind === "unpair"
                : disabled
            }
            explanation={
              session
                ? "End this keyboard and mouse connection."
                : "Move your pointer across a screen edge to control this device with your keyboard and mouse."
            }
            onClick={() =>
              session
                ? disconnect()
                : peer.address
                  ? connect(peer.id, peer)
                  : setSettings(true)
            }
          >
            {session ? <Square size={18} /> : <Keyboard size={18} />}
            <span>
              {session
                ? session.phase === "disconnecting"
                  ? "Disconnecting…"
                  : transitioning
                    ? "Cancel"
                    : active
                      ? "Stop sharing"
                      : "Disconnect"
                : "Share keyboard & mouse"}
            </span>
          </ExplainedButton>
          <ExplainedButton
            className="device-action"
            aria-disabled="true"
            explanation="View and control the other device’s desktop from here. Coming later."
          >
            <Monitor size={18} />
            <span>Remote desktop</span>
          </ExplainedButton>
        </div>
      )}
      {active && (
        <div className="flex flex-wrap items-center gap-3 border-t border-line px-5 py-3">
          <span className="mr-auto text-xs text-muted">Screen edge</span>
          <Select
            label="Screen edge"
            compact
            value={peer.edge}
            disabled={pending}
            onChange={(value) =>
              changeEdge(peer.id, { ...peer, edge: value as Device["edge"] })
            }
            options={[
              { value: "left", label: "Left" },
              { value: "right", label: "Right" },
            ]}
          />
        </div>
      )}
      {settings && (
        <Dialog
          title={confirmUnpair ? `Unpair ${peer.name}?` : "Device settings"}
          onClose={() => {
            if (!busy) {
              setSettings(false);
              setConfirmUnpair(false);
            }
          }}
        >
          {confirmUnpair ? (
            <>
              <p className="text-sm leading-6 text-muted">
                Remove this device’s pairing and access. You’ll need to pair
                again to reconnect.
              </p>
              {error && (
                <p className="error mt-4" role="alert">
                  {error}
                </p>
              )}
              <div className="mt-6 flex justify-end gap-2">
                <button
                  className="button quiet"
                  disabled={busy}
                  onClick={() => {
                    setConfirmUnpair(false);
                    setError("");
                  }}
                >
                  Cancel
                </button>
                <button
                  className="button destructive"
                  disabled={busy || disabled}
                  onClick={async () => {
                    setBusy(true);
                    setError("");
                    try {
                      await api.unpair(peer.id);
                      await refresh();
                    } catch (e) {
                      setError(String(e));
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  Unpair
                </button>
              </div>
            </>
          ) : (
            <form
              className="space-y-4"
              onSubmit={async (e) => {
                e.preventDefault();
                setBusy(true);
                try {
                  await api.update(peer.id, device);
                  await refresh();
                  setSettings(false);
                } catch (e) {
                  setError(String(e));
                } finally {
                  setBusy(false);
                }
              }}
            >
              <label className="field">
                Name
                <input
                  required
                  maxLength={100}
                  value={device.name}
                  onChange={(e) =>
                    setDevice({ ...device, name: e.target.value })
                  }
                />
              </label>
              <details className="text-xs text-muted">
                <summary className="cursor-pointer">Advanced</summary>
                <label className="field mt-4">
                  Address
                  <input
                    required
                    spellCheck={false}
                    value={device.address}
                    placeholder="192.168.1.20:48177"
                    onChange={(e) =>
                      setDevice({ ...device, address: e.target.value })
                    }
                  />
                </label>
                <p className="mt-2 mb-4 text-xs text-muted">
                  Updated automatically when this device is found nearby.
                </p>
                <div className="text-xs text-muted">
                  <h3 className="font-medium">Device identity</h3>
                  <p className="mt-2 select-text break-all font-mono leading-5">
                    {peer.id}
                  </p>
                </div>
              </details>
              {error && (
                <p className="error" role="alert">
                  {error}
                </p>
              )}
              <div className="flex justify-between gap-3">
                <button
                  type="button"
                  className="button destructive-quiet -ms-3.5"
                  disabled={busy || disabled}
                  title={disabled ? "Disconnect before unpairing" : undefined}
                  onClick={() => {
                    setError("");
                    setConfirmUnpair(true);
                  }}
                >
                  <Unlink size={14} />
                  Unpair device
                </button>
                <button className="button primary" disabled={busy}>
                  Save
                </button>
              </div>
            </form>
          )}
        </Dialog>
      )}
    </li>
  );
}
