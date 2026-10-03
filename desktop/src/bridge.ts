import { invoke, isTauri } from "@tauri-apps/api/core";
export const native = isTauri();
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
export type Peer = Device & {
  id: string;
  availability:
    "checking" | "online" | "receiving_off" | "offline" | "update_required";
};
export type Candidate = { name: string; addresses: string[] };
export type Request = {
  id: number;
  kind: "pair" | "verify" | "control";
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
  } | null;
  receiving: boolean;
  code: string | null;
  code_seconds: number;
  approval: Request | null;
  error: string | null;
  permissions: Permissions;
  permission_request: "receive" | "share" | null;
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
  open: (page: "signup" | "account" | "recover") =>
    invoke<void>("account_open_page", { page }),
};
