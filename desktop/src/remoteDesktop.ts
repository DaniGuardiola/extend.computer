import type { Peer } from "./bridge";

const handheldPlatforms = ["android", "ios", "ipados"];

/**
 * Whether Remote desktop means anything on a peer's card: the peer is a
 * computer whose screen could be viewed and controlled from here. Phones and
 * tablets, and devices that said they can't share their screen, have no
 * desktop to view, so their cards leave the tile out. A device of unknown
 * type keeps it.
 */
export function hasDesktopToView(
  peer: Pick<Peer, "device_info" | "offers">,
  /** The reported platform, or the account directory's when none was reported. */
  platform?: string,
) {
  const kind = peer.device_info?.kind;
  if (kind === "phone" || kind === "tablet") return false;
  if (peer.offers && !peer.offers.includes("display-source-mirror"))
    return false;
  if (kind === "desktop" || kind === "laptop") return true;
  return !handheldPlatforms.includes(platform?.toLowerCase() ?? "");
}
