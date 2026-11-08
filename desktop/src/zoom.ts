/**
 * Zoom steps for Cmd/Ctrl + plus, minus and 0, as in a browser. 150% still
 * leaves the window's minimum width about 390 points of layout, the narrow
 * width the screens are designed for.
 */
export const zoomLevels = [0.8, 0.9, 1, 1.1, 1.25, 1.5] as const;
export const zoomStorageKey = "extend.computer.zoom";
export type ZoomAction = "in" | "out" | "reset";

type ZoomKey = Pick<
  KeyboardEvent,
  "key" | "code" | "metaKey" | "ctrlKey" | "altKey"
>;

/** Command on a Mac and Control elsewhere, with plus (or =), minus or 0. */
export function zoomAction(event: ZoomKey, mac: boolean): ZoomAction | null {
  const modifier = mac
    ? event.metaKey && !event.ctrlKey
    : event.ctrlKey && !event.metaKey;
  if (!modifier || event.altKey) return null;
  if (event.key === "+" || event.key === "=" || event.code === "NumpadAdd")
    return "in";
  if (event.key === "-" || event.key === "_" || event.code === "NumpadSubtract")
    return "out";
  if (event.key === "0" || event.code === "Numpad0") return "reset";
  return null;
}

/** The next step in that direction, staying within the steps. */
export function nextZoom(current: number, action: ZoomAction): number {
  if (action === "reset") return 1;
  const levels: readonly number[] = zoomLevels;
  return action === "in"
    ? (levels.find((level) => level > current + 0.001) ?? levels.at(-1)!)
    : ([...levels].reverse().find((level) => level < current - 0.001) ??
        levels[0]);
}

/** A saved zoom, or 100% when it's missing or not one of the steps. */
export function parseZoom(saved: string | null): number {
  const value = Number(saved);
  return (zoomLevels as readonly number[]).includes(value) ? value : 1;
}
