import { invoke, isTauri } from "@tauri-apps/api/core";
export const native = isTauri();
// The OS only shapes wording ("this Mac" on macOS); `Snapshot.capabilities`
// decides what is offered.
const os = navigator.platform;
export const localNoun = os.startsWith("Mac") ? "Mac" : "computer";
export const unavailableHere = `Not available on ${os.startsWith("Win") ? "Windows" : `this ${localNoun}`} yet.`;
/** What this device can start, and which OS grants apply to it. */
export type Capabilities = {
  extend_display: boolean;
  mirror_display: boolean;
  share_input: boolean;
  /** Show and control another device's screen here. */
  remote_desktop: boolean;
  /** Accessibility approval for keyboard and mouse. */
  input_permissions: boolean;
  wifi_optimization: boolean;
};
export type Permissions = {
  listen: boolean;
  post: boolean;
  available: boolean;
  wifi: boolean;
  wifi_pending: boolean;
  wifi_installing: boolean;
  wifi_setup_failed: boolean;
};
export type LocalDeviceInfo = {
  name: string;
  identity: string;
  version: string;
};
export type Device = { name: string; address: string; edge: "left" | "right" };
export type DeviceKind = "desktop" | "laptop" | "tablet" | "phone" | "unknown";
export type DeviceInfo = { platform: string; kind: DeviceKind };
export type DisplayConfig = {
  mode: "extend" | "mirror";
  width: number;
  height: number;
  fps: number;
  bitrate: number;
  retina: boolean;
  auto_size: boolean;
  codec: "h264" | "hevc";
  separate_cursor: boolean;
};
export type Link = "wifi" | "ethernet" | "other";
/** Live connection quality over the last few seconds of a session. */
export type Quality = {
  level: "good" | "fair" | "poor";
  latency_ms?: number;
  loss_percent?: number;
  degraded_seconds: number;
};
export type Peer = Device & {
  id: string;
  device_info?: DeviceInfo | null;
  trust_source: "pairing" | "account";
  route?: "local" | "internet" | "usb" | null;
  /** The peer's own physical link, reported over the local network. */
  link?: Link;
  /** Roles the peer offered, such as `display-viewer`; null when it didn't say. */
  offers?: string[] | null;
  availability:
    "checking" | "online" | "receiving_off" | "offline" | "update_required";
};
/** A peer that didn't list its roles (an older build) is allowed everything. */
export const peerOffers = (peer: Peer, role: string) =>
  !peer.offers || peer.offers.includes(role);
export type Candidate = { name: string; addresses: string[] };
export type Request = {
  id: number;
  kind: "pair" | "verify";
  peer: string;
  symbols: number[] | null;
};
export type Snapshot = {
  development: boolean;
  discoverable: boolean;
  addresses: string[];
  peers: Peer[];
  removed_peers: { id: string; name: string; reason: string }[];
  notification: { id: number; message: string } | null;
  session: {
    id: number;
    kind: "incoming" | "outgoing" | "pair" | "unpair";
    phase: "connecting" | "approval" | "connected" | "disconnecting";
    peer: string | null;
    route?: "local" | "internet" | "usb" | null;
    /** This device's physical link for a local network session. */
    link?: Link;
    quality?: Quality;
    reconnecting?: boolean;
    display?: "extend" | "mirror" | null;
    display_config?: DisplayConfig | null;
  } | null;
  receiving: boolean;
  code: string | null;
  code_seconds: number;
  approval: Request | null;
  error: string | null;
  /** The device `error` belongs to; its card shows it instead of the banner. */
  error_peer?: string | null;
  permissions: Permissions;
  permission_request: "receive" | "share" | null;
  capabilities: Capabilities;
};
export const api = {
  setLocalDeviceName: (name: string) =>
    invoke<void>("set_local_device_name", { name }),
  localDeviceInfo: () => invoke<LocalDeviceInfo>("local_device_info"),
  snapshot: () => invoke<Snapshot>("snapshot"),
  discover: () => invoke<Candidate[]>("discover"),
  pair: (device: Device, code: string) =>
    invoke<void>("pair_device", { device, code }),
  pairNearby: (device: Device) => invoke<void>("pair_nearby", { device }),
  closePairing: () => invoke<void>("close_pairing"),
  connect: (peer: string, device: Device) =>
    invoke<void>("connect_device", { peer, device }),
  display: (
    peer: string,
    device: Device,
    mode: "extend" | "mirror",
    config?: DisplayConfig,
    /** Sizes an extended display offers up front, so resizing only switches modes. */
    presets?: [number, number][],
  ) =>
    invoke<void>("connect_display", {
      peer,
      device,
      presets,
      config: config ?? {
        mode,
        width: 1440,
        height: 900,
        fps: 60,
        bitrate: 20_000_000,
        retina: true,
        auto_size: true,
        codec: "hevc",
        separate_cursor: true,
      },
    }),
  updateDisplay: (session: number, config: DisplayConfig) =>
    invoke<void>("update_display", { session, config }),
  usbDevices: () => invoke<UsbDevice[]>("usb_devices"),
  connectUsb: (peer: string, vendor: string, serial: string) =>
    invoke<void>("connect_usb", { peer, vendor, serial }),
  disconnectUsb: () => invoke<void>("disconnect_usb"),
  usbStatus: () => invoke<UsbStatus>("usb_status"),
  disconnect: () => invoke<void>("disconnect"),
  receive: (pairing: boolean) => invoke<void>("start_receiving", { pairing }),
  stopReceiving: () => invoke<void>("stop_receiving"),
  answer: (id: number, answer: "deny" | "remember") =>
    invoke<void>("answer_request", { id, answer }),
  update: (peer: string, device: Device) =>
    invoke<void>("update_device", { peer, device }),
  unpair: (peer: string) => invoke<void>("unpair_device", { peer }),
  dismissRemoved: (peer: string) => invoke<void>("dismiss_removed", { peer }),
  dismissNotification: (id: number) =>
    invoke<void>("dismiss_notification", { id }),
  checkPermissions: () => invoke<Permissions>("check_permissions"),
  openPermission: (permission: "listen" | "post" | "wifi") =>
    invoke<void>("open_permission", { permission }),
  dismiss: () => invoke<void>("dismiss_message"),
};

export type AccountDevice = {
  id: string;
  name: string;
  platform: string;
  fingerprint: string;
  online: boolean;
  last_seen: number | null;
};
export type AccountState = {
  server: string;
  email: string | null;
  device_id: string | null;
  devices: AccountDevice[];
  pending: { totp: boolean; keys: boolean; recovery: boolean } | null;
  browser_pending: boolean;
  error: string | null;
};
export const accounts = {
  status: () => invoke<AccountState>("account_status"),
  refresh: () => invoke<AccountState>("account_refresh"),
  configure: (server: string) =>
    invoke<AccountState>("account_configure", { server }),
  login: (email: string, password: string) =>
    invoke<AccountState>("account_login", { email, password }),
  verify: (code: string) => invoke<AccountState>("account_verify", { code }),
  logout: () => invoke<AccountState>("account_logout"),
  remove: (id: string) => invoke<AccountState>("account_remove_device", { id }),
  cancel: () => invoke<AccountState>("account_cancel"),
  browser: () => invoke<AccountState>("account_browser_login"),
  /** Sign in with a passkey, or answer the second-factor step with one. */
  passkey: (secondFactor = false) =>
    invoke<AccountState>("account_passkey", { secondFactor }),
  /** Only builds associated with extend.computer can use passkeys natively. */
  passkeyAvailable: () => invoke<boolean>("account_passkey_available"),
  open: (page: "signup" | "account" | "recover" | "account-servers") =>
    invoke<void>("account_open_page", { page }),
};

export type UsbDevice = {
  vendor: string;
  serial: string;
  name: string;
  accessory: boolean;
};
export type UsbStatus = {
  state: string;
  peer?: string;
  signaling_route?: string;
  error?: string;
};
