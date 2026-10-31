import { useEffect, useState, type ReactNode } from "react";
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
  Lightbulb,
} from "lucide-react";
import {
  api,
  localNoun,
  native,
  unavailableHere,
  type Capabilities,
  type Device,
  type Link,
  type Peer,
  type Snapshot,
} from "./bridge";
import { DeviceIcon } from "./DeviceIcon";
import { TruncatedText } from "./TruncatedText";
import { Select } from "./Select";
import { UsbConnection } from "./UsbConnection";
import { Dialog } from "./Dialog";
import { ExplainedButton } from "./ExplainedButton";
import {
  displayConfig,
  presetDisplaySizes,
  readDisplayPreferences,
} from "./displayPreferences";
import { DisplayConnectionSettings } from "./DisplayConnectionSettings";
import {
  QualityBars,
  QualityDetails,
  qualitySuggestion,
} from "./ConnectionQuality";
const availabilityStyle = {
  update_required: { label: "Update required", dot: "bg-status-attention" },
  online: { label: "Online", dot: "bg-emerald-500" },
  receiving_off: {
    label: "Not accepting connections",
    dot: "bg-status-attention",
  },
  offline: { label: "Offline", dot: "bg-muted/60" },
  checking: {
    label: "Checking…",
    dot: "bg-muted/60 motion-safe:animate-pulse",
  },
};

export function DeviceRow({
  peer,
  platform,
  capabilities,
  disabled,
  connect,
  session,
  pending,
  disconnect,
  changeEdge,
  refresh,
  sessionError,
}: {
  peer: Peer;
  platform?: string;
  /** What this computer can start; other actions stay visible but explained. */
  capabilities: Capabilities;
  disabled: boolean;
  session: Snapshot["session"];
  pending: boolean;
  disconnect: () => void;
  changeEdge: (peer: string, device: Device) => void;
  connect: (peer: string, device: Device) => void;
  refresh: () => Promise<void>;
  /** A session failure reported by the app for this device. */
  sessionError?: string;
}) {
  const connected = session?.phase === "connected";
  const active = connected && session?.kind === "outgoing";
  const sharingInput = active && !session?.display;
  const transitioning = !!session && !connected;
  const connectionLabel = connected
    ? session?.kind === "incoming"
      ? "Connected (incoming)"
      : "Connected"
    : session?.phase === "disconnecting"
      ? "Disconnecting…"
      : session?.kind === "unpair"
        ? "Unpairing…"
        : session?.reconnecting
          ? "Reconnecting…"
          : "Connecting…";
  const status =
    availabilityStyle[active ? "online" : peer.availability] ??
    availabilityStyle.checking;
  const route = session?.route ?? peer.route;
  const sessionRoute = connected ? session?.route : undefined;
  const localLink = sessionRoute === "local" ? session?.link : undefined;
  const peerLink = sessionRoute === "local" ? peer.link : undefined;
  const routeLabel =
    sessionRoute === "usb"
      ? "USB"
      : sessionRoute === "local"
        ? localLinkLabel(localLink, peerLink)
        : sessionRoute === "internet"
          ? "Internet relay"
          : undefined;
  const routeExplanation =
    sessionRoute === "usb"
      ? `Connected to ${peer.name} directly over a USB cable.`
      : sessionRoute === "local"
        ? localExplanation(peer.name, localLink, peerLink)
        : `Connected to ${peer.name} through the internet relay. A local network or USB cable is faster.`;
  const quality = connected ? session?.quality : undefined;
  const suggestion =
    connected && session ? qualitySuggestion(peer, session) : undefined;
  const canManageUsb =
    native &&
    navigator.platform.startsWith("Mac") &&
    (!platform || platform.toLowerCase() === "android");
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
  const sessionMode = session ? (session.display ?? "input") : undefined;
  const unavailable = (icon: ReactNode, label: string) => (
    <ExplainedButton
      className="device-action"
      aria-disabled="true"
      explanation={unavailableHere}
    >
      {icon}
      <span>{label}</span>
    </ExplainedButton>
  );
  const stopAction = session && (
    <ExplainedButton
      className={`device-action ${active ? "bg-line/40" : ""}`}
      disabled={
        pending ||
        session.phase === "disconnecting" ||
        session.kind === "unpair"
      }
      explanation="End this connection."
      onClick={disconnect}
    >
      <Square size={18} />
      <span>
        {session.phase === "disconnecting"
          ? "Disconnecting…"
          : transitioning
            ? "Cancel"
            : active
              ? session.display === "mirror"
                ? "Stop mirroring"
                : session.display
                  ? "Stop extending"
                  : "Stop sharing"
              : "Disconnect"}
      </span>
    </ExplainedButton>
  );
  return (
    <li
      className={`device-card overflow-hidden rounded-xl border ${connected ? "device-connected" : "border-line bg-surface/50"} ${(error || sessionError) && !settings ? "device-card-error" : ""}`}
    >
      <div className="flex items-start gap-3 px-5 pt-4 pb-3">
        <DeviceIcon
          platform={platform}
          kind={peer.device_info?.kind}
          size={20}
          className={`shrink-0 ${connected ? "text-success" : "text-muted"}`}
        />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <TruncatedText className="text-sm font-medium">
              {peer.name}
            </TruncatedText>
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
                  ? "Signed in to your account."
                  : "Paired with this computer."}
              </Tooltip>
            </TooltipProvider>
          </div>
          <div
            className={`mt-1 flex min-h-7 flex-wrap items-center gap-x-3 gap-y-2 text-xs ${connected ? "text-success" : "text-muted"}`}
          >
            {session ? (
              <>
                <TooltipProvider timeout={400}>
                  <TooltipAnchor
                    render={
                      <span
                        tabIndex={routeLabel ? 0 : undefined}
                        className={`inline-flex items-center gap-1.5 rounded-full px-2 py-1 font-medium ${connected ? "bg-success/15" : "bg-line/50"} ${routeLabel ? "cursor-help" : ""}`}
                      />
                    }
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
                    {routeLabel && (
                      <span className="font-normal opacity-80">
                        · {routeLabel}
                      </span>
                    )}
                    {quality && <QualityBars level={quality.level} />}
                  </TooltipAnchor>
                  {routeLabel && (
                    <Tooltip className="explanation-tooltip">
                      {quality && <QualityDetails quality={quality} />}
                      <span className="block">{routeExplanation}</span>
                    </Tooltip>
                  )}
                </TooltipProvider>
                <span className="inline-flex items-center gap-1.5">
                  {session.display ? (
                    <Monitor size={14} aria-hidden="true" />
                  ) : (
                    <Keyboard size={14} aria-hidden="true" />
                  )}
                  {session.display === "extend"
                    ? "Extended display"
                    : session.display === "mirror"
                      ? "Mirrored display"
                      : "Keyboard & mouse"}
                </span>
              </>
            ) : (
              <span className="inline-flex items-center gap-1.5">
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
              </span>
            )}
            {canManageUsb ? (
              <UsbConnection
                peer={peer}
                route={route}
                hasSession={disabled || !!session}
                refresh={refresh}
              />
            ) : route === "usb" && !routeLabel ? (
              <span>USB</span>
            ) : null}
          </div>
        </div>
        <button
          className="device-settings icon-button -me-2.25 -mt-1.5"
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
          {sessionMode === "extend" ? (
            stopAction
          ) : !capabilities.extend_display ? (
            unavailable(<MonitorUp size={18} />, "Extend display")
          ) : (
            <ExplainedButton
              className="device-action"
              disabled={disabled || !!session || busy}
              explanation="Extend your desktop onto this device."
              onClick={async () => {
                setBusy(true);
                setError("");
                try {
                  await api.display(
                    peer.id,
                    peer,
                    "extend",
                    displayConfig("extend", readDisplayPreferences(peer.id)),
                    presetDisplaySizes(peer.id),
                  );
                  await refresh();
                } catch (e) {
                  setError(e instanceof Error ? e.message : String(e));
                } finally {
                  setBusy(false);
                }
              }}
            >
              <MonitorUp size={18} />
              <span>Extend display</span>
            </ExplainedButton>
          )}
          {sessionMode === "mirror" ? (
            stopAction
          ) : !capabilities.mirror_display ? (
            unavailable(<Copy size={18} />, "Mirror display")
          ) : (
            <ExplainedButton
              className="device-action"
              disabled={disabled || !!session || busy}
              explanation="Show your screen on this device."
              onClick={async () => {
                setBusy(true);
                setError("");
                try {
                  await api.display(
                    peer.id,
                    peer,
                    "mirror",
                    displayConfig("mirror", readDisplayPreferences(peer.id)),
                  );
                  await refresh();
                } catch (e) {
                  setError(e instanceof Error ? e.message : String(e));
                } finally {
                  setBusy(false);
                }
              }}
            >
              <Copy size={18} />
              <span>Mirror display</span>
            </ExplainedButton>
          )}
          {sessionMode === "input" ? (
            stopAction
          ) : !capabilities.share_input ? (
            unavailable(<Keyboard size={18} />, "Share keyboard & mouse")
          ) : (
            <ExplainedButton
              className="device-action"
              disabled={disabled || !!session || busy}
              explanation="Control this device with your keyboard and mouse."
              onClick={() =>
                peer.address || peer.route === "usb"
                  ? connect(peer.id, peer)
                  : setSettings(true)
              }
            >
              <Keyboard size={18} />
              <span>Share keyboard &amp; mouse</span>
            </ExplainedButton>
          )}
          <ExplainedButton
            className="device-action"
            aria-disabled="true"
            explanation={
              capabilities.remote_desktop
                ? "View and control the other device’s desktop from here. Coming later."
                : unavailableHere
            }
          >
            <Monitor size={18} />
            <span>Remote desktop</span>
          </ExplainedButton>
        </div>
      )}
      {sharingInput && (
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
      {active && session.display && session.display_config && (
        <DisplayConnectionSettings
          key={session.id}
          peer={peer.id}
          session={session.id}
          current={session.display_config}
          pending={pending}
          refresh={refresh}
          onError={setError}
        />
      )}
      {suggestion && !error && (
        <p className="device-tip" role="status">
          <Lightbulb size={14} aria-hidden="true" className="mt-0.5 shrink-0" />
          {suggestion}
        </p>
      )}
      {(error || sessionError) && !settings && (
        <p className="device-error" role="alert">
          {error || sessionError}
        </p>
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
                      setError(e instanceof Error ? e.message : String(e));
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
                setError("");
                try {
                  await api.update(peer.id, device);
                  await refresh();
                  setSettings(false);
                } catch (e) {
                  setError(e instanceof Error ? e.message : String(e));
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
              <details className="text-xs text-muted" open={!peer.address}>
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

const linkName = { wifi: "Wi-Fi", ethernet: "Ethernet" } as const;
/** Name the slowest known link; Wi-Fi on either end limits the session. */
function localLinkLabel(local?: Link, remote?: Link) {
  if (local === "wifi" || remote === "wifi") return "Wi-Fi";
  if (local === "ethernet" && remote === "ethernet") return "Ethernet";
  return "Local network";
}
function side(link?: Link) {
  return link === "wifi" || link === "ethernet"
    ? `on ${linkName[link]}`
    : link === "other"
      ? "on another network adapter"
      : undefined;
}
function localExplanation(peer: string, local?: Link, remote?: Link) {
  const mine = side(local);
  const theirs = side(remote);
  const base = `Connected to ${peer} over your local network.`;
  if (mine && theirs)
    return local === remote
      ? `${base} Both devices are ${mine}.`
      : `${base} This ${localNoun} is ${mine} and ${peer} is ${theirs}.`;
  if (mine) return `${base} This ${localNoun} is ${mine}.`;
  if (theirs) return `${base} ${peer} is ${theirs}.`;
  return `${base} It may use Wi-Fi or Ethernet.`;
}
