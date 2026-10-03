import { lazy, Suspense, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { native } from "./bridge";
const ReleaseHistory = lazy(() => import("./ReleaseHistory"));

type Settings = {
  available: boolean;
  automatic_checks: boolean;
  automatic_downloads: boolean;
  channel: "stable" | "beta" | "alpha" | "canary";
  version: string;
};

export function Updates() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [history, setHistory] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const refresh = () => {
    if (native)
      void invoke<Settings>("update_settings")
        .then(setSettings)
        .catch((e) => setError(String(e)));
  };
  useEffect(() => {
    refresh();
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, []);
  async function configure(next: Settings) {
    setBusy(true);
    setError("");
    try {
      setSettings(
        await invoke<Settings>("configure_updates", {
          checks: next.automatic_checks,
          downloads: next.automatic_downloads,
          channel: next.channel,
        }),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function check() {
    setBusy(true);
    setError("");
    try {
      await invoke("check_for_updates");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section aria-labelledby="updates-heading" className="my-8">
      <h2 id="updates-heading" className="mb-4 text-sm font-medium">
        Updates
      </h2>
      <div className="flex flex-wrap items-center gap-3">
        {settings && (
          <span className="text-sm text-muted">Version {settings.version}</span>
        )}
        <button
          className="button"
          disabled={!settings?.available || busy}
          onClick={() => void check()}
        >
          Check for updates…
        </button>
        <button className="button quiet" onClick={() => setHistory(true)}>
          What’s new
        </button>
      </div>
      {settings?.available ? (
        <fieldset disabled={busy} className="mt-4 grid gap-3 text-sm">
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              checked={settings.automatic_checks}
              onChange={(e) =>
                void configure({
                  ...settings,
                  automatic_checks: e.target.checked,
                })
              }
            />
            Automatically check for updates
          </label>
          <label className="flex items-center gap-2">
            <input
              type="checkbox"
              disabled={!settings.automatic_checks}
              checked={settings.automatic_downloads}
              onChange={(e) =>
                void configure({
                  ...settings,
                  automatic_downloads: e.target.checked,
                })
              }
            />
            Automatically download and install updates when sharing has finished
          </label>
          <label className="flex items-center gap-3">
            Update channel
            <select
              className="rounded border border-line bg-transparent p-2"
              value={settings.channel}
              onChange={(e) =>
                void configure({
                  ...settings,
                  channel: e.target.value as Settings["channel"],
                })
              }
            >
              <option value="stable">Stable</option>
              <option value="beta">Beta</option>
              <option value="alpha">Alpha</option>
              <option value="canary">Canary</option>
            </select>
          </label>
          <p className="text-xs text-muted">
            Preview channels may contain unfinished features. Returning to
            Stable waits for a newer stable release; it does not downgrade this
            installation.
          </p>
        </fieldset>
      ) : (
        <p className="mt-3 text-xs text-muted">
          Automatic updates are unavailable in this development build.
        </p>
      )}
      {error && (
        <p role="alert" className="mt-3 text-sm">
          {error}
        </p>
      )}
      {history && (
        <Suspense fallback={<p role="status">Loading release history…</p>}>
          <ReleaseHistory
            version={settings?.version}
            onClose={() => setHistory(false)}
          />
        </Suspense>
      )}
    </section>
  );
}
