import type { DisplayConfig } from "./bridge";

export type DisplayPreferences = {
  resolution: string;
  retina: boolean;
  fps: number;
  quality: "responsive" | "balanced" | "detail";
};
export const displayDefaults: DisplayPreferences = {
  resolution: "auto",
  retina: true,
  fps: 60,
  quality: "balanced",
};
const bitrates = {
  responsive: 8_000_000,
  balanced: 20_000_000,
  detail: 40_000_000,
};

export function preferencesFromConfig(
  current: DisplayConfig,
): DisplayPreferences {
  return {
    resolution: current.auto_size
      ? "auto"
      : `${current.width}x${current.height}`,
    retina: current.retina,
    fps: current.fps,
    quality:
      current.bitrate === bitrates.responsive
        ? "responsive"
        : current.bitrate === bitrates.detail
          ? "detail"
          : "balanced",
  };
}

// The bridge reports applied profiles, not the receiver's complete hardware
// mode list. Offer smaller stream sizes and frame-rate limits from profiles
// already negotiated in this connection; do not invent higher capabilities.
export function connectionDisplayOptions(
  initial: DisplayConfig,
  current: DisplayConfig = initial,
) {
  // A fixed ladder from the largest profile seen, so choosing a size never
  // adds new choices below it.
  const largest = [initial, current].reduce((a, b) =>
    b.width * b.height > a.width * a.height ? b : a,
  );
  const sizes = new Set<string>();
  for (const ratio of [1, 0.875, 0.75, 0.625, 0.5]) {
    // Scaled sizes stay even for the encoder; the full size is kept as is.
    const even = (value: number) =>
      ratio === 1 ? value : Math.round((value * ratio) / 2) * 2;
    const width = even(largest.width);
    const height = even(largest.height);
    if (width >= 640 && height >= 480) sizes.add(`${width}x${height}`);
  }
  for (const profile of [initial, current])
    sizes.add(`${profile.width}x${profile.height}`);
  const maxFPS = Math.max(initial.fps, current.fps);
  return {
    sizes: [...sizes].sort(
      (a, b) => Number(b.split("x")[0]) - Number(a.split("x")[0]),
    ),
    frameRates: [...new Set([30, 60, 120, initial.fps, current.fps])]
      .filter((fps) => fps <= maxFPS)
      .sort((a, b) => a - b),
    retina: initial.retina || current.retina,
  };
}

export function readDisplayPreferences(peer: string): DisplayPreferences {
  try {
    const p = JSON.parse(localStorage.getItem(`display:${peer}`) ?? "null");
    if (
      p &&
      typeof p.retina === "boolean" &&
      Number.isInteger(p.fps) &&
      p.fps >= 30 &&
      p.fps <= 120 &&
      Object.hasOwn(bitrates, p.quality) &&
      typeof p.resolution === "string" &&
      (p.resolution === "auto" || /^\d{3,4}x\d{3,4}$/.test(p.resolution))
    ) {
      const result = {
        resolution: p.resolution,
        retina: p.retina,
        fps: p.fps,
        quality: p.quality,
      };
      displayConfig("extend", result);
      return result;
    }
  } catch {
    /* Fall back if stored settings are unavailable or invalid. */
  }
  return { ...displayDefaults };
}
/**
 * The largest profile seen with this device. Applying a new size restarts the
 * display session, so the choices are anchored here rather than to the
 * session's first profile, which would be whatever size was last applied.
 */
export function largestDisplayProfile(
  peer: string,
  current: DisplayConfig,
): DisplayConfig {
  const key = `display-largest:${peer}`;
  let largest = current;
  try {
    const saved = JSON.parse(localStorage.getItem(key) ?? "null");
    if (
      saved &&
      [saved.width, saved.height, saved.fps].every(Number.isInteger) &&
      typeof saved.retina === "boolean"
    )
      largest = {
        ...current,
        width:
          saved.width * saved.height > current.width * current.height
            ? saved.width
            : current.width,
        height:
          saved.width * saved.height > current.width * current.height
            ? saved.height
            : current.height,
        fps: Math.max(saved.fps, current.fps),
        retina: saved.retina || current.retina,
      };
    const { width, height, fps, retina } = largest;
    localStorage.setItem(key, JSON.stringify({ width, height, fps, retina }));
  } catch {
    /* Without storage, choices follow this session's profile. */
  }
  return largest;
}
export function saveDisplayPreferences(
  peer: string,
  value: DisplayPreferences,
) {
  try {
    localStorage.setItem(`display:${peer}`, JSON.stringify(value));
  } catch {
    /* The applied session remains valid when storage is unavailable. */
  }
}
export function displayConfig(
  mode: DisplayConfig["mode"],
  value: DisplayPreferences,
  current?: DisplayConfig,
): DisplayConfig {
  const [width, height] =
    value.resolution === "auto"
      ? [current?.width ?? 1440, current?.height ?? 900]
      : value.resolution.split("x").map(Number);
  const scale = value.retina ? 2 : 1;
  if (
    !Number.isInteger(width) ||
    !Number.isInteger(height) ||
    !Number.isInteger(value.fps) ||
    value.fps < 30 ||
    value.fps > 120 ||
    !Object.hasOwn(bitrates, value.quality) ||
    width < 640 ||
    height < 480 ||
    width * scale > 4096 ||
    height * scale > 4096
  )
    throw new Error("Choose a smaller resolution for Retina.");
  return {
    mode,
    width,
    height,
    fps: value.fps,
    bitrate: bitrates[value.quality],
    retina: value.retina,
    auto_size: value.resolution === "auto",
    codec: current?.codec ?? "hevc",
    separate_cursor: current?.separate_cursor ?? true,
  };
}
