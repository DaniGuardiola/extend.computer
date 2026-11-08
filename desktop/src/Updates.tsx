import { lazy, Suspense, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { native } from "./bridge";
const ReleaseHistory = lazy(() => import("./ReleaseHistory"));

type Channel = "stable" | "beta" | "alpha" | "canary";
type Settings = {
  available: boolean;
  /** Why automatic updates are off, when they are. */
  unavailable: string | null;
  automatic_checks: boolean;
  automatic_downloads: boolean;
  channel: Channel;
  /** Channels with published releases, plus the current choice. */
  channels: Channel[];
  /** False when the chosen channel has no releases to update from. */
  channel_published: boolean;
  version: string;
};
const channelNames: Record<Channel, string> = {
  stable: "Stable",
  beta: "Beta",
  alpha: "Alpha",
  canary: "Canary",
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
          className="button secondary"
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
          {/* Only channels with releases are offered, so Stable alone needs no choice. */}
          {settings.channels.length > 1 && (
            <>
              <label className="flex items-center gap-3">
                Update channel
                <select
                  className="rounded border border-line bg-transparent p-2"
                  value={settings.channel}
                  onChange={(e) =>
                    void configure({
                      ...settings,
                      channel: e.target.value as Channel,
                    })
                  }
                >
                  {settings.channels.map((channel) => (
                    <option key={channel} value={channel}>
                      {channelNames[channel]}
                    </option>
                  ))}
                </select>
              </label>
              <p className="text-xs text-muted">
                {settings.channel_published
                  ? "Preview channels may contain unfinished features. Returning to Stable waits for a newer stable release; it does not downgrade this installation."
                  : `${channelNames[settings.channel]} has no releases yet. Choose Stable to keep getting updates.`}
              </p>
            </>
          )}
        </fieldset>
      ) : settings?.unavailable || !native ? (
        // The app says why; a browser preview has no updater at all.
        <p className="mt-3 text-xs text-muted">
          {settings?.unavailable ??
            "Automatic updates are unavailable in this development build."}
        </p>
      ) : null}
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
