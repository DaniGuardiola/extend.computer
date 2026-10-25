import { useState } from "react";
import { ChevronDown, SlidersHorizontal } from "lucide-react";
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
    <details className="group border-t border-line">
      <summary
        className="flex cursor-pointer list-none items-center gap-2.5 px-5 py-3.5 text-sm font-medium hover:bg-line/30 focus-visible:bg-line/30 [&::-webkit-details-marker]:hidden"
        onClick={(event) => {
          if (busy) event.preventDefault();
          else if (!event.currentTarget.parentElement?.hasAttribute("open"))
            setValue(preferencesFromConfig(current));
        }}
      >
        <SlidersHorizontal
          size={16}
          aria-hidden="true"
          className="text-muted"
        />
        {current.mode === "mirror"
          ? "Mirrored display settings"
          : "Extended display settings"}
        <ChevronDown
          size={16}
          aria-hidden="true"
          className="ms-auto text-muted motion-safe:transition-transform group-open:rotate-180"
        />
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
          <p className="text-xs text-muted">
            Now: {current.width} × {current.height}
            {current.retina ? " Retina" : ""} · {current.fps} FPS · up to{" "}
            {current.bitrate / 1_000_000} Mbps
          </p>
          <button
            className="button primary shrink-0"
            disabled={busy || pending}
          >
            {busy ? "Applying…" : "Apply"}
          </button>
        </div>
      </form>
    </details>
  );
}
