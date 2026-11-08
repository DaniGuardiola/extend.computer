import { useEffect, useState } from "react";
import { api, native, statusArea } from "./bridge";

/** Opening at login, like closing the window, leaves only the status icon,
 * so other devices can still connect. */
export function OpenAtLogin() {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (native)
      void api
        .openAtLogin()
        .then(setEnabled)
        .catch((e) => setError(String(e)));
  }, []);
  async function change(next: boolean) {
    setBusy(true);
    setError("");
    try {
      setEnabled(await api.setOpenAtLogin(next));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section aria-labelledby="startup-heading" className="mb-8">
      <h2 id="startup-heading" className="mb-4 text-sm font-medium">
        Startup
      </h2>
      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={enabled ?? false}
          disabled={enabled === null || busy}
          onChange={(e) => void change(e.target.checked)}
        />
        Open at login
      </label>
      <p className="mt-2 text-xs text-muted">
        extend.computer starts in the {statusArea}, so your devices can connect.
        Closing the window also keeps it running there.
      </p>
      {error && (
        <p role="alert" className="mt-3 text-sm">
          {error}
        </p>
      )}
    </section>
  );
}
