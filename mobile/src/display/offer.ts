import type { LaunchSession } from "../../modules/extend-display";

// Called only from a focused, foreground, terminal display screen. The two
// native snapshots must describe the same new offer; a replaced/stale offer
// must never rebind the current viewer or carry its input grant forward.
export function reconnectOffer(
  previousId: string | undefined,
  media: { state?: unknown; id?: unknown },
  launch: LaunchSession,
): LaunchSession | null {
  return media.state === "offered" &&
    typeof media.id === "string" && media.id.length > 0 &&
    media.id !== previousId && launch.available && launch.id === media.id
    ? launch
    : null;
}
