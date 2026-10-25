import type { Link, Peer, Quality, Snapshot } from "./bridge";

const levelText = {
  good: "Good connection",
  fair: "Fair connection",
  poor: "Poor connection",
} as const;
const levelColor = {
  good: "text-success",
  fair: "text-status-attention",
  poor: "text-danger",
} as const;

/** Three signal bars; fewer light up as the connection gets worse. */
export function QualityBars({ level }: { level: Quality["level"] }) {
  const lit = level === "good" ? 3 : level === "fair" ? 2 : 1;
  return (
    <svg
      viewBox="0 0 12 10"
      className={`h-2.5 w-3 shrink-0 ${levelColor[level]}`}
      role="img"
      aria-label={levelText[level]}
    >
      {[0, 1, 2].map((bar) => (
        <rect
          key={bar}
          x={bar * 4.5}
          y={6 - bar * 3}
          width={3}
          height={4 + bar * 3}
          rx={1}
          className="transition-opacity duration-500"
          fill="currentColor"
          opacity={bar < lit ? 1 : 0.25}
        />
      ))}
    </svg>
  );
}

/** Live figures for the connection tooltip. */
export function QualityDetails({ quality }: { quality: Quality }) {
  const figures = [
    quality.latency_ms !== undefined &&
      `${quality.latency_ms < 1 ? "<1" : quality.latency_ms} ms round trip`,
    quality.loss_percent !== undefined &&
      `${quality.loss_percent}% packets lost`,
  ].filter(Boolean);
  return (
    <span className="mb-1.5 flex items-center gap-1.5 font-medium">
      <QualityBars level={quality.level} />
      {levelText[quality.level]}
      {figures.length > 0 && (
        <span className="font-normal opacity-80">· {figures.join(" · ")}</span>
      )}
    </span>
  );
}

type Session = NonNullable<Snapshot["session"]>;
/** Seconds below good before suggesting changes, so brief dips stay quiet. */
const SUGGEST_AFTER = 5;

/** A practical next step for a connection that has stayed below good. */
export function qualitySuggestion(peer: Peer, session: Session) {
  const quality = session.quality;
  if (!quality || quality.level === "good") return undefined;
  if (quality.degraded_seconds < SUGGEST_AFTER) return undefined;
  const display = session.display
    ? " Lowering the frame rate or video quality in display settings also helps."
    : "";
  if (session.route === "internet")
    return `This connection goes through the internet relay. Join both devices to the same network, or connect them with a USB cable.${display}`;
  if (session.route === "usb")
    return `The USB connection is unsteady. Try another cable, or plug it straight into the Mac instead of a hub.${display}`;
  const wifi = wifiSide(peer.name, session.link, peer.link);
  if (wifi)
    return `${wifi} Ethernet or a USB cable between the devices gives a steadier connection.${display}`;
  return `Your local network may be busy. A USB cable connects the devices directly.${display}`;
}

function wifiSide(peer: string, local?: Link, remote?: Link) {
  if (local === "wifi" && remote === "wifi")
    return "Both devices are on Wi-Fi.";
  if (local === "wifi") return "This Mac is on Wi-Fi.";
  if (remote === "wifi") return `${peer} is on Wi-Fi.`;
  return undefined;
}
