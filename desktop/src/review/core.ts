// Loaded only by Vite's ui-review mode. No native calls or network requests.
import type {
  AccountState,
  Candidate,
  DisplayConfig,
  DeviceInfo,
  Peer,
  Quality,
  Snapshot,
  UsbDevice,
  UsbStatus,
} from "../bridge";
export const isTauri = () => true;
export const scenarioId =
  new URLSearchParams(location.search).get("state") ?? "empty";
const id = scenarioId;
const pending = <T>(): Promise<T> => new Promise(() => {});
const fingerprint = (n: number) => n.toString(16).padStart(64, "a");
const peer = (
  n: number,
  name: string,
  availability: Peer["availability"] = "online",
): Peer => ({
  id: fingerprint(n),
  name,
  availability,
  address: `192.168.1.${20 + n}:48177`,
  edge: "left",
  trust_source: "pairing",
  route: "local",
  device_info: { platform: "macos", kind: "laptop" },
});
const emptyIds = [
  "empty",
  "receiving-on",
  "startup-loading",
  "startup-error",
  "account-empty",
  "account-one",
];
export const snapshot: Snapshot = {
  development: false,
  discoverable: false,
  addresses: ["192.168.1.10:48177"],
  peers: emptyIds.includes(id) ? [] : [peer(1, "Studio Mac")],
  removed_peers: [],
  notification: null,
  session: null,
  receiving: id === "receiving-on",
  code: null,
  code_seconds: 120,
  approval: null,
  error: null,
  permissions: {
    listen: true,
    post: true,
    available: true,
    wifi: true,
    wifi_pending: false,
    wifi_installing: false,
    wifi_setup_failed: false,
  },
  permission_request: null,
};
const availabilities: Peer["availability"][] = [
  "online",
  "receiving_off",
  "checking",
  "offline",
  "update_required",
];
if (["many", "many-online", "session-many"].includes(id))
  snapshot.peers = Array.from({ length: 15 }, (_, i) =>
    peer(
      i + 1,
      i === 0 ? "Studio Mac" : `Desk PC ${String(i + 1).padStart(2, "0")}`,
      id === "many" ? availabilities[i % 5] : "online",
    ),
  );
if (id === "two")
  snapshot.peers.push({
    ...peer(2, "Pixel Tablet"),
    route: "usb",
    device_info: { platform: "android", kind: "tablet" },
  });
if (id === "desktop-mac") snapshot.peers[0].device_info!.kind = "desktop";
if (id === "android-phone")
  snapshot.peers[0] = {
    ...peer(1, "Pixel Phone"),
    device_info: { platform: "android", kind: "phone" },
  };
if (id.startsWith("usb-") || id === "permission-usb")
  snapshot.peers[0].device_info = { platform: "android", kind: "tablet" };
if (["many", "many-online", "session-many"].includes(id)) {
  const deviceInfos: DeviceInfo[] = [
    { platform: "macos", kind: "laptop" },
    { platform: "macos", kind: "desktop" },
    { platform: "windows", kind: "laptop" },
    { platform: "linux", kind: "desktop" },
    { platform: "android", kind: "tablet" },
    { platform: "android", kind: "phone" },
  ];
  snapshot.peers.forEach((device, i) => {
    device.device_info = deviceInfos[i % deviceInfos.length];
    if (i > 0 && i < 14) {
      const names = [
        "MacBook",
        "Studio Mac",
        "Work laptop",
        "Desk PC",
        "Pixel Tablet",
        "Pixel Phone",
      ];
      device.name = `${names[i % names.length]} ${String(i + 1).padStart(2, "0")}`;
    }
  });
}
if (id === "unknown-platform") delete snapshot.peers[0].device_info;
if (id === "account-trust") snapshot.peers[0].trust_source = "account";
if (id === "unicode-name")
  snapshot.peers[0].name = "工作室 · portátil de diseño 🖥️";
if (id === "missing-address") snapshot.peers[0].address = "";
if (id === "long-name")
  snapshot.peers[0].name =
    "Design studio · MacBook Pro with a very long descriptive name for layout review";
if (["offline", "receiving-off", "checking", "update-required"].includes(id))
  snapshot.peers[0].availability = (
    {
      offline: "offline",
      "receiving-off": "receiving_off",
      checking: "checking",
      "update-required": "update_required",
    } as const
  )[id as "offline"];
if (id === "removed") {
  snapshot.peers = [];
  snapshot.removed_peers = [
    {
      id: fingerprint(1),
      name: "Studio Mac",
      reason: "Studio Mac removed this pairing. Pair again to reconnect.",
    },
  ];
}
if (id === "notification")
  snapshot.notification = {
    id: 1,
    message: "Pairing was declined on the other device.",
  };
if (id === "global-error") {
  snapshot.error =
    "Could not connect to Studio Mac. Check that both devices are online and try again.";
  snapshot.error_peer = fingerprint(1);
}
if (id === "long-error") {
  snapshot.error =
    "Connection could not be established. The device at 192.168.1.21:48177 stopped responding during the encrypted handshake. Check your network, keep extend.computer open on both devices, and enable Allow connections on the receiving device before trying again.";
  snapshot.error_peer = fingerprint(1);
}
if (id === "app-error")
  snapshot.error =
    "Could not verify macOS permissions. Retry Allow connections; if this continues, reopen or reinstall extend.computer.";
if (id === "approve-code")
  snapshot.approval = {
    id: 1,
    kind: "pair",
    peer: fingerprint(2),
    symbols: null,
  };
if (["approve-symbols", "approval-error"].includes(id))
  snapshot.approval = {
    id: 1,
    kind: "verify",
    peer: fingerprint(2),
    symbols: [0, 4, 8, 12],
  };
if (id.startsWith("permission-")) {
  snapshot.permissions.post = false;
  snapshot.permissions.listen = false;
  snapshot.permissions.wifi = false;
  if (id === "permission-helper") snapshot.permissions.available = false;
  if (id === "permission-wifi-installing") {
    snapshot.permissions.post = true;
    snapshot.permissions.wifi_installing = true;
  }
  if (id === "permission-wifi-failed") {
    snapshot.permissions.post = true;
    snapshot.permissions.wifi_setup_failed = true;
  }
  if (id === "permission-restart") {
    snapshot.permissions.post = true;
    snapshot.permissions.wifi = true;
  }
  if (id === "permission-usb") snapshot.peers[0].route = "usb";
}
if (["manual-no-address", "settings-no-address"].includes(id))
  snapshot.addresses = [];
const config = (mode: "extend" | "mirror"): DisplayConfig => ({
  mode,
  width: 1440,
  height: 900,
  fps: 60,
  bitrate: 20_000_000,
  retina: true,
  auto_size: true,
  codec: "hevc",
  separate_cursor: true,
});
const sessionIds = [
  "input-usb",
  "display-connecting",
  "display-reconnecting",
  "display-disconnecting",
  "display-save-error",
  "connecting",
  "approval-wait",
  "reconnecting",
  "disconnecting",
  "unpairing",
  "input-local",
  "input-incoming",
  "input-internet",
  "input-wifi",
  "input-ethernet",
  "display-poor",
  "extend",
  "mirror",
  "display-incoming",
  "session-many",
  "device-active-input",
  "device-active-display",
  "usb-session",
];
if (sessionIds.includes(id)) {
  const display = [
    "extend",
    "device-active-display",
    "usb-session",
    "display-connecting",
    "display-reconnecting",
    "display-disconnecting",
    "display-save-error",
    "display-poor",
  ].includes(id)
    ? "extend"
    : ["mirror", "display-incoming"].includes(id)
      ? "mirror"
      : null;
  snapshot.session = {
    id: 1,
    peer: fingerprint(1),
    kind:
      id === "unpairing"
        ? "unpair"
        : id.endsWith("incoming")
          ? "incoming"
          : "outgoing",
    phase:
      (id.endsWith("connecting") && !id.endsWith("disconnecting")) ||
      id === "unpairing"
        ? "connecting"
        : id.endsWith("disconnecting")
          ? "disconnecting"
          : id === "approval-wait"
            ? "approval"
            : "connected",
    reconnecting: id.endsWith("reconnecting"),
    route:
      id === "input-internet"
        ? "internet"
        : ["usb-session", "input-usb"].includes(id)
          ? "usb"
          : "local",
    link:
      id === "input-wifi"
        ? "wifi"
        : id === "input-ethernet"
          ? "ethernet"
          : undefined,
    display,
    display_config: display ? config(display) : null,
  };
  if (id === "input-wifi" || id === "input-ethernet")
    snapshot.peers[0].link = "ethernet";
  const quality: Record<string, Quality> = {
    "input-local": { level: "good", latency_ms: 6, degraded_seconds: 0 },
    "input-ethernet": { level: "good", latency_ms: 2, degraded_seconds: 0 },
    "input-wifi": { level: "poor", latency_ms: 140, degraded_seconds: 12 },
    "input-internet": { level: "fair", latency_ms: 70, degraded_seconds: 8 },
    "input-usb": { level: "good", latency_ms: 1, degraded_seconds: 0 },
    extend: {
      level: "good",
      latency_ms: 3,
      loss_percent: 0,
      degraded_seconds: 0,
    },
    "display-poor": {
      level: "poor",
      latency_ms: 24,
      loss_percent: 6.4,
      degraded_seconds: 10,
    },
  };
  if (quality[id] && snapshot.session.phase === "connected")
    snapshot.session.quality = quality[id];
}
if (id.startsWith("usb-")) {
  snapshot.peers[0].name = "Studio Mac";
}
if (
  [
    "usb-connected",
    "usb-session",
    "usb-disconnect-error",
    "usb-disconnecting",
  ].includes(id)
)
  snapshot.peers[0].route = "usb";
const signedIn = [
  "account-trust",
  "account-long-email",
  "settings-account",
  "account-signed-in",
  "account-custom-server",
  "account-registration",
  "account-empty",
  "account-one",
  "account-many",
  "account-presence-error",
  "account-remove",
  "account-remove-error",
  "account-logout-error",
  "account-refresh-error",
].includes(id);
export const accountState: AccountState = {
  server: "https://extend.computer",
  email: signedIn ? "review@example.test" : null,
  device_id: signedIn && id !== "account-registration" ? "this-device" : null,
  devices: [],
  pending: null,
  browser_pending: id === "account-browser",
  error: null,
};
if (id === "account-custom-server")
  accountState.server = "https://accounts.example.test";
if (id === "account-long-email")
  accountState.email =
    "design.studio.shared.account+desktop.header.review@devices.example.test";
if (
  [
    "account-mfa",
    "account-mfa-error",
    "account-recovery",
    "account-mfa-busy",
    "account-mfa-key-error",
  ].includes(id)
)
  accountState.pending = {
    totp: id !== "account-recovery",
    keys: id !== "account-recovery",
    recovery: true,
  };
accountState.devices = snapshot.peers.map((p) => ({
  id: p.id,
  name: p.name,
  fingerprint: p.id,
  platform: p.device_info?.platform ?? "macos",
  online: p.availability === "online",
  last_seen: null,
}));
if (id === "unknown-platform") accountState.devices = [];
if (signedIn)
  accountState.devices.push({
    id: "this-device",
    name: "Review MacBook",
    fingerprint: fingerprint(0),
    platform: "macos",
    online: true,
    last_seen: null,
  });
if (
  [
    "account-one",
    "account-remove",
    "account-remove-error",
    "account-presence-error",
    "account-refresh-error",
  ].includes(id)
)
  accountState.devices.push({
    id: "travel",
    name: "Travel laptop",
    fingerprint: fingerprint(90),
    platform: "windows",
    online: true,
    last_seen: null,
  });
if (id === "account-many")
  accountState.devices.push(
    ...Array.from({ length: 15 }, (_, i) => ({
      id: `account-${i}`,
      name: `Account device ${i + 1}`,
      fingerprint: fingerprint(90 + i),
      platform: ["macos", "windows", "linux", "android"][i % 4],
      online: i % 3 !== 0,
      last_seen: null,
    })),
  );
if (id === "account-presence-error")
  accountState.error = "Account server is temporarily unavailable.";
if (id === "account-empty") snapshot.peers = [];
let localName = "Review MacBook";
let usbStatus: UsbStatus = { state: "disconnected" };
if (
  [
    "usb-connected",
    "usb-session",
    "usb-disconnect-error",
    "usb-disconnecting",
  ].includes(id)
)
  usbStatus = { state: "connected", peer: fingerprint(1) };
if (id === "two") usbStatus = { state: "connected", peer: fingerprint(2) };
if (id === "usb-connecting") usbStatus = { state: "connecting" };
if (id === "usb-occupied")
  usbStatus = { state: "connected", peer: fingerprint(3) };
if (id === "usb-disconnected")
  usbStatus = {
    state: "failed",
    peer: fingerprint(1),
    error: "The USB cable was disconnected.",
  };
let updateSettings = {
  available: id !== "updates-unavailable",
  automatic_checks: id !== "updates-manual",
  automatic_downloads: id !== "updates-manual",
  channel: "stable",
  version: "0.2.1",
};
function fail(message: string): never {
  throw new Error(message);
}
function begin(peerId: string, display?: DisplayConfig) {
  snapshot.session = {
    id: 1,
    peer: peerId,
    kind: "outgoing",
    phase: "connected",
    route: snapshot.peers.find((p) => p.id === peerId)?.route ?? "local",
    display: display?.mode,
    display_config: display,
  };
}
let notificationDismissed = false;
document.addEventListener(
  "click",
  (event) => {
    if (
      event.target instanceof Element &&
      event.target.closest('[aria-label="Dismiss notification"]')
    )
      notificationDismissed = true;
  },
  true,
);
export async function invoke<T>(
  command: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  let result: unknown;
  switch (command) {
    case "snapshot":
      if (id === "startup-loading") return pending<T>();
      if (id === "startup-error")
        fail("Could not load devices. Restart extend.computer and try again.");
      result = structuredClone(snapshot);
      break;
    case "account_status":
      if (id === "account-loading") return pending<T>();
      if (id === "account-status-error")
        fail("Could not load account status. Try again.");
      result = structuredClone(accountState);
      break;
    case "account_refresh":
      if (id === "account-refresh-error")
        fail("Could not refresh account devices. Try again.");
      result = structuredClone(accountState);
      break;
    case "check_permissions":
      if (id === "permission-checking") return pending<T>();
      if (id === "permission-error")
        fail("Could not check permissions. Try again.");
      result = { ...snapshot.permissions };
      break;
    case "open_permission":
      if (id === "permission-guide-error")
        fail(
          "Could not open Accessibility settings. Try opening System Settings manually.",
        );
      if (args.permission === "wifi") {
        snapshot.permissions.wifi = true;
        snapshot.permissions.wifi_setup_failed = false;
      } else {
        snapshot.permissions.post = true;
        snapshot.permissions.listen = true;
      }
      break;
    case "start_receiving":
      snapshot.receiving = true;
      if (args.pairing) {
        snapshot.discoverable = id !== "pair-starting";
        snapshot.code = id === "manual-expired" ? null : "a1b2-c3d4-e5f6-7890";
        if (id === "pair-wait")
          snapshot.session = {
            id: 1,
            peer: fingerprint(2),
            kind: "pair",
            phase: "approval",
          };
      }
      break;
    case "stop_receiving":
      snapshot.receiving = false;
      break;
    case "close_pairing":
      snapshot.discoverable = false;
      break;
    case "discover":
      if (id === "pair-error")
        fail(
          "Nearby discovery failed. Check your local network or pair manually.",
        );
      result = (
        id === "pair-many"
          ? Array.from({ length: 15 }, (_, i) => ({
              name: `Nearby device ${i + 1}`,
              addresses: [`192.168.2.${i + 1}:48177`],
            }))
          : ["pair-one", "pair-nearby-error"].includes(id)
            ? [{ name: "Nearby MacBook", addresses: ["192.168.2.1:48177"] }]
            : []
      ) satisfies Candidate[];
      break;
    case "pair_device":
      if (id === "manual-busy") return pending<T>();
      if (id === "manual-error")
        fail(
          "Pairing code is invalid or expired. Ask the other device for a new code.",
        );
      snapshot.peers.push(
        peer(2, String((args.device as { name: string }).name)),
      );
      break;
    case "pair_nearby":
      if (id === "pair-nearby-error")
        fail("Nearby pairing could not start. Try again.");
      snapshot.approval = {
        id: 1,
        peer: fingerprint(2),
        kind: "verify",
        symbols: [0, 4, 8, 12],
      };
      break;
    case "answer_request":
      if (id === "approval-error")
        fail("Pairing approval could not be sent. Try again.");
      if (args.answer === "remember")
        snapshot.peers.push(peer(2, "Newly paired Mac"));
      snapshot.approval = null;
      snapshot.session = null;
      break;
    case "connect_device":
      begin(String(args.peer));
      break;
    case "connect_display":
      if (id === "row-error")
        fail(
          "Display could not start. Allow Screen Recording in System Settings and try again.",
        );
      begin(String(args.peer), args.config as DisplayConfig);
      break;
    case "update_display":
      if (id === "display-save-error")
        fail("Display configuration could not be applied. Try again.");
      if (snapshot.session)
        snapshot.session.display_config = args.config as DisplayConfig;
      break;
    case "disconnect":
      snapshot.session = null;
      break;
    case "update_device":
      if (id === "device-saving") return pending<T>();
      if (id === "device-save-error")
        fail("Device settings could not be saved. Try again.");
      snapshot.peers = snapshot.peers.map((p) =>
        p.id === args.peer ? { ...p, ...(args.device as object) } : p,
      );
      break;
    case "unpair_device":
      if (id === "unpair-busy") return pending<T>();
      if (id === "unpair-error")
        fail("Device could not be unpaired. Try again.");
      snapshot.peers = snapshot.peers.filter((p) => p.id !== args.peer);
      break;
    case "dismiss_message":
      snapshot.error = null;
      break;
    case "dismiss_removed":
      snapshot.removed_peers = [];
      break;
    case "dismiss_notification":
      // Keep toast visible for visual review. Explicit clicks still dismiss it.
      if (notificationDismissed) snapshot.notification = null;
      break;
    case "local_device_info":
      if (id === "settings-name-loading") return pending<T>();
      if (id === "settings-info-error")
        fail("Could not load device information. Try again.");
      result = { name: localName, identity: fingerprint(0), version: "0.2.1" };
      break;
    case "set_local_device_name":
      if (id === "settings-name-saving") return pending<T>();
      if (id === "settings-name-error")
        fail("Could not save device name. Try again.");
      localName = String(args.name).trim() || "Review MacBook";
      break;
    case "update_settings":
      result = { ...updateSettings };
      break;
    case "configure_updates":
      updateSettings = {
        ...updateSettings,
        automatic_checks: Boolean(args.checks),
        automatic_downloads: Boolean(args.downloads),
        channel: String(args.channel),
      };
      result = { ...updateSettings };
      break;
    case "check_for_updates":
      if (id === "updates-error")
        fail("Update server could not be reached. Try again later.");
      if (id === "updates-busy") return pending<T>();
      snapshot.notification = { id: 2, message: "You’re up to date." };
      break;
    case "account_configure":
      accountState.server = String(args.server);
      result = { ...accountState };
      break;
    case "account_login":
      if (id === "account-error")
        fail("Email or password is incorrect. Try again.");
      if (id === "account-busy") return pending<T>();
      accountState.email = String(args.email);
      accountState.device_id = "this-device";
      result = { ...accountState };
      break;
    case "account_verify":
      if (id === "account-mfa-busy") return pending<T>();
      if (id === "account-mfa-error")
        fail("Verification code is incorrect. Try again.");
      accountState.pending = null;
      accountState.email = "review@example.test";
      accountState.device_id = "this-device";
      result = { ...accountState };
      break;
    case "account_passkey_available":
      result = true;
      break;
    case "account_passkey":
      if (id === "account-passkey-error" || id === "account-mfa-key-error")
        fail("Passkey sign-in was canceled.");
      accountState.pending = null;
      accountState.email = "review@example.test";
      accountState.device_id = "this-device";
      result = { ...accountState };
      break;
    case "account_browser_login":
      if (id === "account-browser-error")
        fail("Browser sign-in could not start. Try again.");
      accountState.browser_pending = true;
      result = { ...accountState };
      break;
    case "account_cancel":
      accountState.browser_pending = false;
      accountState.pending = null;
      result = { ...accountState };
      break;
    case "account_logout":
      if (id === "account-logout-error") fail("Could not sign out. Try again.");
      accountState.email = null;
      result = { ...accountState };
      break;
    case "account_remove_device":
      if (id === "account-remove-error")
        fail("Account device could not be removed. Try again.");
      accountState.devices = accountState.devices.filter(
        (d) => d.id !== args.id,
      );
      result = { ...accountState };
      break;
    case "account_open_page":
      break; // No external navigation in review.
    case "usb_devices":
      if (id === "usb-scanning") return pending<T>();
      if (id === "usb-error")
        fail("USB device scan failed. Check your cable and try again.");
      result = (
        [
          "usb-found",
          "usb-many",
          "usb-accessory",
          "usb-selected",
          "usb-connect-error",
        ].includes(id)
          ? Array.from({ length: id === "usb-many" ? 5 : 1 }, (_, i) => ({
              vendor: "18d1",
              serial: `REVIEW-CABLE-${i + 1}`,
              name: i === 0 ? "Pixel Tablet" : `Android device ${i + 1}`,
              accessory: id === "usb-accessory",
            }))
          : []
      ) satisfies UsbDevice[];
      break;
    case "usb_status":
      result = { ...usbStatus };
      break;
    case "connect_usb":
      if (id === "usb-connect-error")
        fail(
          "USB connection could not be established. Unplug and reconnect the cable.",
        );
      usbStatus = { state: "connected", peer: String(args.peer) };
      snapshot.peers.find((peer) => peer.id === args.peer)!.route = "usb";
      break;
    case "disconnect_usb":
      if (id === "usb-disconnect-error")
        fail("USB could not be disconnected. Try again.");
      if (id === "usb-disconnecting") return pending<T>();
      for (const peer of snapshot.peers)
        if (peer.id === usbStatus.peer) peer.route = "local";
      usbStatus = { state: "disconnected" };
      break;
    default:
      fail(`UI review has no fixture for native command: ${command}`);
  }
  return result as T;
}
