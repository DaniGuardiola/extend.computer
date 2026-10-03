import { useEffect, useState } from "react";
import { Laptop, LoaderCircle, ChevronRight } from "lucide-react";
import { Dialog } from "./Dialog";
import { ManualPairDialog } from "./ManualPairDialog";
import { api, type Candidate, type Snapshot } from "./bridge";

export function PairDialog({
  snapshot,
  onClose,
  refresh,
}: {
  snapshot: Snapshot;
  onClose: () => void;
  refresh: () => Promise<void>;
}) {
  const [manual, setManual] = useState(false);
  const [candidates, setCandidates] = useState<Candidate[]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const scan = async () => {
      try {
        const found = await api.discover();
        if (!stopped) {
          setCandidates(found);
          setError("");
        }
      } catch (e) {
        if (!stopped) setError(String(e));
      } finally {
        if (!stopped) {
          timer = setTimeout(scan, 5000);
        }
      }
    };
    void scan();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, []);
  if (manual)
    return (
      <ManualPairDialog
        snapshot={snapshot}
        refresh={refresh}
        onClose={() => setManual(false)}
      />
    );
  const nearby = candidates.filter(
    (c) =>
      c.addresses.length &&
      !c.addresses.some(
        (a) =>
          snapshot.addresses.includes(a) ||
          snapshot.peers.some((p) => p.address === a),
      ),
  );
  return (
    <Dialog title="Pair a device" onClose={onClose}>
      <p role="status" className="text-sm leading-6 text-muted">
        {snapshot.discoverable
          ? "Your device is visible nearby."
          : snapshot.session
            ? "Pairing in progress…"
            : "Making this device visible…"}
      </p>
      {(error || snapshot.error) && (
        <p className="error mt-4" role="alert">
          {error || snapshot.error}
        </p>
      )}
      <p className="mt-5 flex items-center gap-2 text-xs text-muted">
        <span
          className="size-1.5 shrink-0 rounded-full bg-current motion-safe:animate-pulse"
          role="img"
          aria-label="Searching for nearby devices"
        />
        Nearby devices
      </p>
      <div className="mt-2 max-h-64 space-y-2 overflow-auto">
        {nearby.map((c) => (
          <button
            key={c.addresses.join(",")}
            className="flex w-full items-center gap-3 rounded-xl border border-line p-4 text-left hover:bg-line/40 focus-visible:bg-line/40 disabled:opacity-50"
            disabled={busy || !!snapshot.session}
            onClick={async () => {
              setBusy(true);
              setError("");
              try {
                await api.pairNearby({
                  name: c.name,
                  address: c.addresses[0],
                  edge: "left",
                });
                await refresh();
              } catch (e) {
                setError(String(e));
              } finally {
                setBusy(false);
              }
            }}
          >
            <Laptop size={20} />
            <span className="flex-1 truncate text-sm">{c.name}</span>
            <ChevronRight size={16} />
          </button>
        ))}
        {!nearby.length && (
          <p className="py-8 text-center text-sm text-muted">
            Open Pair device on your other device.
          </p>
        )}
      </div>
      {snapshot.session && (
        <p className="mt-4 flex items-center gap-2 text-sm text-muted">
          <LoaderCircle size={14} className="animate-spin" />
          Waiting for both devices to confirm…
        </p>
      )}
      <div className="mt-5 border-t border-line pt-4">
        <button
          className="button quiet -ms-3.5"
          disabled={!!snapshot.session}
          onClick={() => setManual(true)}
        >
          Pair manually
        </button>
      </div>
    </Dialog>
  );
}
