import { useState } from "react";
import { ArrowLeft, LoaderCircle } from "lucide-react";
import { Dialog } from "./Dialog";
import { PairingCodeInput } from "./PairingCodeInput";
import { api, type Snapshot } from "./bridge";
export function ManualPairDialog({
  snapshot,
  onClose,
  refresh,
}: {
  snapshot: Snapshot;
  onClose: () => void;
  refresh: () => Promise<void>;
}) {
  const [mode, setMode] = useState<"join" | "receive">("join");
  const [name, setName] = useState("Other device");
  const [address, setAddress] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const showCode = async () => {
    setBusy(true);
    setError("");
    try {
      await api.receive(true);
      await refresh();
      setMode("receive");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog title="Pair manually" onClose={onClose}>
      {error && (
        <p role="alert" className="error mb-4">
          {error}
        </p>
      )}
      {mode === "receive" ? (
        <div className="space-y-5">
          <p className="text-sm text-muted">
            On the other device, open extend.computer and enter this code.
          </p>
          <div className="rounded-xl border border-line bg-canvas px-4 py-6 text-center">
            <p className="select-text font-mono text-xl tracking-wide">
              {snapshot.code || "Code expired or used"}
            </p>
            {snapshot.code && (
              <p className="mt-3 text-xs text-muted">
                Expires in {snapshot.code_seconds}s · One use
              </p>
            )}
          </div>
          {snapshot.addresses.map((address) => (
            <p
              key={address}
              className="select-text text-center font-mono text-xs text-muted"
            >
              {address}
            </p>
          ))}
          <div className="flex items-center justify-between">
            <button className="button quiet" onClick={() => setMode("join")}>
              <ArrowLeft size={14} />
              Back
            </button>
            <button
              className="button primary"
              disabled={busy || !!snapshot.session}
              onClick={() => void showCode()}
            >
              New code
            </button>
          </div>
          <p className="text-xs leading-5 text-muted">
            You’ll approve the pairing here. Paired devices can connect while
            Allow connections is on.
          </p>
        </div>
      ) : (
        <form
          className="space-y-4"
          onSubmit={async (e) => {
            e.preventDefault();
            setBusy(true);
            setError("");
            try {
              await api.pair({ name, address, edge: "left" }, code.trim());
              await refresh();
              onClose();
            } catch (e) {
              setError(String(e));
            } finally {
              setBusy(false);
            }
          }}
        >
          <label className="field">
            Device name
            <input
              required
              maxLength={100}
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          <label className="field">
            Device address
            <input
              required
              autoComplete="off"
              spellCheck={false}
              placeholder="192.168.1.20:48177"
              value={address}
              onChange={(e) => setAddress(e.target.value)}
            />
          </label>
          <label className="field">
            Pairing code
            <PairingCodeInput value={code} onChange={setCode} />
          </label>
          <p className="text-xs leading-5 text-muted">
            On the other device, choose Pair device → Pair manually → Show my
            code.
          </p>
          <div className="flex items-center justify-between pt-2">
            <button
              type="button"
              className="button quiet"
              disabled={busy}
              onClick={() => void showCode()}
            >
              Show my code
            </button>
            <button
              className="button primary"
              disabled={busy || !!snapshot.session}
            >
              {busy && <LoaderCircle size={14} className="animate-spin" />}Pair
              device
            </button>
          </div>
        </form>
      )}
    </Dialog>
  );
}
