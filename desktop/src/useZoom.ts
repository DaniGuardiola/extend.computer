import { useEffect } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { native } from "./bridge";
import { nextZoom, parseZoom, zoomAction, zoomStorageKey } from "./zoom";

let level: number | undefined;
function current() {
  if (level === undefined) {
    try {
      level = parseZoom(localStorage.getItem(zoomStorageKey));
    } catch {
      level = 1;
    }
  }
  return level;
}
/** Scales the whole webview, like a browser's page zoom, so layouts reflow. */
function apply(value: number) {
  // Window chrome drawn by the OS doesn't zoom; see `.app-header`.
  document.documentElement.style.setProperty("--app-zoom", String(value));
  if (native) void getCurrentWebview().setZoom(value).catch(console.error);
}

/** Applies the remembered zoom; call once before the first render. */
export function restoreZoom() {
  if (current() !== 1) apply(current());
}

/** Cmd (Ctrl on Windows) with plus, minus or 0 zooms in, out, or back to 100%. */
export function useZoomKeys() {
  useEffect(() => {
    const mac = navigator.platform.startsWith("Mac");
    const keydown = (event: KeyboardEvent) => {
      const action = zoomAction(event, mac);
      if (!action) return;
      event.preventDefault();
      const next = nextZoom(current(), action);
      if (next === current()) return;
      level = next;
      apply(next);
      try {
        localStorage.setItem(zoomStorageKey, String(next));
      } catch {
        // Zoom still applies; it just isn't remembered.
      }
    };
    window.addEventListener("keydown", keydown);
    return () => window.removeEventListener("keydown", keydown);
  }, []);
}
