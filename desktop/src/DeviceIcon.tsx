import {
  Laptop,
  Monitor,
  RectangleHorizontal,
  Smartphone,
  Tablet,
  TabletSmartphone,
} from "lucide-react";
import type { DeviceKind } from "./bridge";

export function DeviceIcon({
  platform,
  kind,
  size,
  className,
}: {
  platform?: string;
  kind?: DeviceKind;
  size?: number;
  className?: string;
}) {
  const normalizedPlatform = platform?.toLowerCase();
  const explicitIcon = {
    laptop: Laptop,
    desktop: Monitor,
    tablet: Tablet,
    phone: Smartphone,
  };
  const Icon =
    kind && kind !== "unknown"
      ? explicitIcon[kind]
      : normalizedPlatform === "android" ||
          normalizedPlatform === "ios" ||
          normalizedPlatform === "ipados"
        ? TabletSmartphone
        : normalizedPlatform === "macos" ||
            normalizedPlatform === "darwin" ||
            normalizedPlatform === "windows" ||
            normalizedPlatform === "linux"
          ? Monitor
          : RectangleHorizontal;

  return (
    <Icon
      size={size}
      className={kind === "tablet" ? `rotate-90 ${className ?? ""}` : className}
      aria-hidden="true"
    />
  );
}
