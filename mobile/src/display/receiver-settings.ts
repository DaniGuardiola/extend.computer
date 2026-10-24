import type { ReceiverDisplayConfig, ReceiverDisplaySettings, ReceiverSettingsStatus } from "../../modules/extend-display";

export function settingsFromConfig(config: ReceiverDisplayConfig): ReceiverDisplaySettings {
  return { width: config.width, height: config.height, auto_size: config.auto_size, fps: config.fps, bitrate: config.bitrate };
}
export function sizeValue(settings: ReceiverDisplaySettings): string {
  return settings.auto_size ? "fit" : `${settings.width}x${settings.height}`;
}
export function sizeOptions(config: ReceiverDisplayConfig, draft: ReceiverDisplaySettings) {
  const scale = config.retina ? 2 : 1;
  const sizes = [[config.width, config.height], [draft.width, draft.height], [1280, 800], [1920, 1200], [1920, 1080], [2560, 1600]];
  const seen = new Set<string>();
  return [{ value: "fit", label: "Fit display" }, ...sizes.flatMap(([width, height]) => {
    const value = `${width}x${height}`;
    if (seen.has(value) || width * scale > 4096 || height * scale > 4096) return [];
    seen.add(value);
    return [{ value, label: `${width} × ${height}` }];
  })];
}
export function chooseSize(draft: ReceiverDisplaySettings, value: string): ReceiverDisplaySettings {
  if (value === "fit") return { ...draft, auto_size: true };
  const [width, height] = value.split("x").map(Number);
  return { ...draft, width, height, auto_size: false };
}
export function matchingCompletion(status: ReceiverSettingsStatus, request: number | null) {
  return request !== null && status.result?.id === request ? status.result : null;
}
export function observedRequest(status: ReceiverSettingsStatus, request: number | null): number | null {
  return request ?? status.pending;
}
export function appliedDraft(status: ReceiverSettingsStatus, request: number | null): ReceiverDisplaySettings | null {
  const result = matchingCompletion(status, request);
  return result?.outcome === "applied" && status.applied?.revision === result.revision
    ? settingsFromConfig(status.applied.config) : null;
}
export function formatConfig(config: ReceiverDisplayConfig): string {
  return `${config.width} × ${config.height} · ${config.fps} FPS · ${config.bitrate / 1_000_000} Mbps`;
}
