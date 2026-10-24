import { useEffect, useRef, useState } from "react";
import { Cable, LoaderCircle } from "lucide-react";
import {
  api,
  native,
  type Peer,
  type UsbDevice,
  type UsbStatus,
} from "./bridge";
import { Dialog } from "./Dialog";
import { Select } from "./Select";

// Selecting a USB descriptor only selects a cable. The existing paired peer
// still authenticates signaling and pinned encrypted media independently.
export function UsbConnection({
  peer,
  route = peer.route,
  hasSession,
  refresh,
}: {
  peer: Peer;
  route?: Peer["route"];
  hasSession: boolean;
  refresh: () => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [devices, setDevices] = useState<UsbDevice[]>([]);
  const [selected, setSelected] = useState("");
  const [status, setStatus] = useState<UsbStatus>({ state: "disconnected" });
  const [scanning, setScanning] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [disconnecting, setDisconnecting] = useState(false);
  const [error, setError] = useState("");
  const [reconnectNeeded, setReconnectNeeded] = useState(false);
  const scanGeneration = useRef(0);
  const cancelled = useRef(false);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const key = (device: UsbDevice) => `${device.vendor}:${device.serial}`;
  const scan = async () => {
    const generation = ++scanGeneration.current;
    setDevices([]);
    setSelected("");
    setScanning(true);
    setError("");
    try {
      const found = await api.usbDevices();
      if (!alive.current || scanGeneration.current !== generation) return;
      setDevices(found);
      setSelected((current) =>
        found.some((d) => key(d) === current) ? current : "",
      );
    } catch (e) {
      if (alive.current && scanGeneration.current === generation)
        setError(String(e));
    } finally {
      if (alive.current && scanGeneration.current === generation)
        setScanning(false);
    }
  };
  useEffect(() => {
    if (!open) return;
    let stopped = false;
    let polling = false;
    const poll = async () => {
      if (polling) return;
      polling = true;
      try {
        const current = await api.usbStatus();
        if (!stopped) setStatus(current);
      } catch (e) {
        if (!stopped) setError(String(e));
      } finally {
        polling = false;
      }
    };
    void poll();
    void scan();
    const timer = window.setInterval(() => void poll(), 1000);
    return () => {
      stopped = true;
      scanGeneration.current++;
      window.clearInterval(timer);
    };
  }, [open]);

  if (!native || !navigator.platform.startsWith("Mac")) return null;
  const ownConnection =
    status.peer === peer.id && status.state !== "disconnected";
  const usbConnected =
    route === "usb" || (open && ownConnection && status.state === "connected");
  const occupied = status.peer && status.peer !== peer.id;
  const starting = connecting || status.state === "connecting";
  const chosen = devices.find((d) => key(d) === selected);
  const disconnect = async () => {
    cancelled.current = true;
    setDisconnecting(true);
    setError("");
    try {
      await api.disconnectUsb();
      if (alive.current) {
        // AOA changes the USB descriptor. An earlier inventory cannot be used
        // again, including a discovery request still running during cancel.
        scanGeneration.current++;
        setDevices([]);
        setSelected("");
        setScanning(false);
        setReconnectNeeded(true);
      }
      const current = await api.usbStatus();
      if (alive.current) setStatus(current);
      await refresh();
    } catch (e) {
      if (alive.current) setError(String(e));
    } finally {
      if (alive.current) setDisconnecting(false);
    }
  };
  return (
    <>
      <button
        type="button"
        className="quiet inline-flex items-center gap-1.5 rounded-md px-1.5 py-0.5 text-xs text-muted"
        aria-label={usbConnected ? "USB connected" : "Connect with USB"}
        onClick={() => {
          setError("");
          setOpen(true);
        }}
      >
        <Cable size={12} aria-hidden="true" />
        {usbConnected ? "USB" : "Connect with USB"}
      </button>
      {open && (
        <Dialog
          title={`USB · ${peer.name}`}
          onClose={() => {
            if (!starting && !disconnecting) setOpen(false);
          }}
        >
          <p className="text-sm leading-6 text-muted">
            USB carries display, keyboard and mouse in either direction. Open
            extend.computer on Android, then select its cable.
          </p>
          {ownConnection ? (
            <p className="mt-4 text-sm" role="status">
              {status.state === "connected"
                ? "USB is ready. Start a display or keyboard and mouse from either device."
                : "USB disconnected. Unplug and reconnect the cable before trying again."}
            </p>
          ) : starting ? (
            <p className="mt-4 flex items-center gap-2 text-sm" role="status">
              <LoaderCircle size={16} className="motion-safe:animate-spin" />
              Connecting… Allow the accessory on your Mac and Android device.
            </p>
          ) : occupied ? (
            <p className="mt-4 text-sm text-muted">
              USB is connected to another device. Disconnect it there first.
            </p>
          ) : (
            <div className="mt-4 space-y-3">
              {devices.length > 0 && (
                <Select
                  label={`USB cable for ${peer.name}`}
                  value={selected}
                  disabled={scanning || hasSession}
                  onChange={setSelected}
                  options={[
                    { value: "", label: "Choose the connected Android device" },
                    ...devices.map((device) => ({
                      value: key(device),
                      label: `${device.name} · ${device.serial}${device.accessory ? " (reconnect cable)" : ""}`,
                    })),
                  ]}
                />
              )}
              <p className="text-xs leading-5 text-muted" role="status">
                {scanning
                  ? "Checking USB devices…"
                  : reconnectNeeded && !chosen
                    ? "USB disconnected. Unplug and reconnect the cable, then choose Check again."
                    : devices.length === 0
                      ? "No compatible Android device found. Plug in a data cable, unlock Android, and allow the accessory on your Mac."
                      : chosen?.accessory
                        ? "Unplug and reconnect this cable to leave its previous accessory connection."
                        : "Choose this tablet or phone’s cable. Its paired identity is checked before USB becomes ready."}
              </p>
              <button
                type="button"
                className="button quiet -ms-3.5"
                disabled={scanning}
                onClick={() => void scan()}
              >
                Check again
              </button>
            </div>
          )}
          {hasSession && !starting && (
            <p className="mt-3 text-xs text-muted">
              Stop the current session before changing the USB connection.
            </p>
          )}
          {(error || (ownConnection && status.error)) && (
            <p className="error mt-4" role="alert">
              {error || status.error}
            </p>
          )}
          <div className="mt-6 flex justify-end gap-2">
            {starting || ownConnection ? (
              <button
                type="button"
                className="button quiet -me-3.5"
                disabled={disconnecting || hasSession}
                onClick={() => void disconnect()}
              >
                {disconnecting
                  ? "Disconnecting…"
                  : starting
                    ? "Cancel connection"
                    : "Disconnect USB"}
              </button>
            ) : (
              <button
                type="button"
                className="button primary"
                disabled={
                  !chosen ||
                  chosen.accessory ||
                  scanning ||
                  hasSession ||
                  !!occupied
                }
                onClick={async () => {
                  if (!chosen) return;
                  scanGeneration.current++;
                  setDevices([]);
                  setSelected("");
                  setScanning(false);
                  setReconnectNeeded(false);
                  cancelled.current = false;
                  setConnecting(true);
                  setError("");
                  try {
                    await api.connectUsb(peer.id, chosen.vendor, chosen.serial);
                    const current = await api.usbStatus();
                    if (alive.current) setStatus(current);
                    await refresh();
                  } catch (e) {
                    if (alive.current && !cancelled.current) {
                      setReconnectNeeded(true);
                      setError(String(e));
                    }
                  } finally {
                    if (alive.current) setConnecting(false);
                  }
                }}
              >
                Connect USB
              </button>
            )}
          </div>
        </Dialog>
      )}
    </>
  );
}
