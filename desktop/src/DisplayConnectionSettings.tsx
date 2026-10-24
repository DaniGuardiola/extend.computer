import { useState } from "react";
import { api, type DisplayConfig } from "./bridge";
import { DisplaySettings } from "./DisplaySettings";
import {
  displayConfig,
  preferencesFromConfig,
  saveDisplayPreferences,
} from "./displayPreferences";

export function DisplayConnectionSettings({
  peer,
  session,
  current,
  pending,
  refresh,
  onError,
}: {
  peer: string;
  session: number;
  current: DisplayConfig;
  pending: boolean;
  refresh: () => Promise<void>;
  onError: (message: string) => void;
}) {
  const [initial] = useState(current);
  const [value, setValue] = useState(() => preferencesFromConfig(current));
  const [busy, setBusy] = useState(false);
  return (
    <details className="border-t border-line">
      <summary
        className="cursor-pointer px-5 py-3 text-xs font-medium"
        onClick={(event) => {
          if (busy) event.preventDefault();
          else if (!event.currentTarget.parentElement?.hasAttribute("open"))
            setValue(preferencesFromConfig(current));
        }}
      >
        Display settings
      </summary>
      <form
        className="space-y-4 px-5 pb-4"
        onSubmit={async (event) => {
          event.preventDefault();
          setBusy(true);
          onError("");
          try {
            await api.updateDisplay(
              session,
              displayConfig(current.mode, value, current),
            );
            saveDisplayPreferences(peer, value);
            await refresh();
          } catch (error) {
            onError(error instanceof Error ? error.message : String(error));
          } finally {
            setBusy(false);
          }
        }}
      >
        <DisplaySettings
          value={value}
          onChange={setValue}
          current={current}
          initial={initial}
          disabled={busy || pending}
        />
        <div className="flex items-center justify-between gap-3">
          <p className="text-xs text-muted">Remembered for this device.</p>
          <button className="button primary" disabled={busy || pending}>
            {busy ? "Applying…" : "Apply"}
          </button>
        </div>
      </form>
    </details>
  );
}
