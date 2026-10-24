import {
  requireNativeModule,
  requireNativeViewManager,
} from "expo-modules-core";
import type { ComponentType, Ref } from "react";
import type { ViewProps } from "react-native";

export type LaunchSession = {
  id?: string;
  available: boolean;
  host: string;
  mode: "extend" | "mirror";
};
export type DeviceAvailability = {
  state: "checking" | "online" | "offline" | "receiving_off" | "unavailable";
  route?: "lan" | "internet" | "usb";
  reason?: string;
};
export type PairedDevice = { peer: string; name: string; address: string; trustSource?: "manual" | "account"; defaultName?: string; authenticatedName?: string | null; label?: string | null };
export type LocalDeviceInfo = { name: string; defaultName: string; customName: boolean; fingerprint: string; version: string; build: string };
export type AccountDevice = { id: string; fingerprint: string; name: string; platform: string; key_verified: boolean; online: boolean };
export type AccountState = { server: string; email: string | null; device_id: string | null; devices: AccountDevice[]; browser_pending: boolean; native_pending: boolean; busy: boolean; loaded: boolean; storage_error: boolean; directory_fresh: boolean; error: string | null };
export type NearbyDevice = {
  id: string;
  name: string;
  addresses: string[];
  pairing: boolean;
};
export type DiscoverySnapshot = {
  state: "idle" | "searching" | "failed";
  candidates: NearbyDevice[];
};
export type ReceivingState = {
  enabled: boolean;
  state: "stopped" | "starting" | "available" | "failed";
  port: number;
};
export type IncomingPairingState = {
  state: "waiting" | "paired" | "rejected" | "expired" | "closed";
  port: number;
  code: string | null;
  secondsRemaining: number;
  addresses: string[];
  request: {
    id: string;
    peer: string;
    kind?: "pair" | "verify";
    symbols?: number[];
  } | null;
};
export type VisualPairingState = {
  id: string;
  state:
    | "connecting"
    | "verify"
    | "paired"
    | "rejected"
    | "failed"
    | "expired"
    | "closed";
  request: {
    id: string;
    peer: string;
    kind: "verify";
    symbols: number[];
  } | null;
  secondsRemaining: number;
  error?: string;
  peer?: string;
};
export type DisplayStatus = {
  state: "connecting" | "streaming" | "disconnected";
  message?: string;
  width?: number;
  height?: number;
};
export type ReceiverDisplayConfig = {
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
export type ReceiverDisplaySettings = Pick<ReceiverDisplayConfig, "width" | "height" | "auto_size" | "fps" | "bitrate">;
export type ReceiverSettingsStatus = {
  id: string;
  supported: boolean;
  closed: boolean;
  available: boolean;
  revision: number;
  config: ReceiverDisplayConfig;
  applied: { revision: number; config: ReceiverDisplayConfig } | null;
  pending: number | null;
  result: { id: number; revision: number; outcome: "applied" | "rejected" | "stale" | "failed" | "closed" | "timed_out"; error: string | null } | null;
};

export const display = requireNativeModule<{
  getAccountState(): Promise<string>;
  resumeAccount(): Promise<void>;
  refreshAccount(): Promise<string>;
  startAccountBrowser(): Promise<string>;
  startAccountPassword(): Promise<void>;
  cancelAccountSignIn(): Promise<string>;
  logoutAccount(): Promise<string>;
  configureAccount(server: string): Promise<string>;
  removeAccountDevice(id: string): Promise<string>;
  openAccountPage(page: "signup" | "account" | "recover"): Promise<void>;
  hasActiveConnection(): boolean;
  getEngineIdentity(): Promise<{ fingerprint: string; publicKey: string }>;
  getLocalDeviceInfo(): Promise<LocalDeviceInfo>;
  setLocalDeviceName(name: string): Promise<LocalDeviceInfo>;
  getPeerDeviceInfo(peer: string): Promise<PairedDevice>;
  setPeerDeviceLabel(peer: string, label: string): Promise<PairedDevice>;
  setPeerDeviceAddress(peer: string, address: string): Promise<PairedDevice>;
  startDiscovery(): void;
  stopDiscovery(): void;
  getNearbyDevices(): DiscoverySnapshot;
  startVisualPairing(address: string, name: string): Promise<string>;
  getVisualPairing(id: string): string;
  respondVisualPairing(id: string, request: string, allow: boolean): void;
  stopVisualPairing(id: string): void;
  startIncomingPairing(): string;
  getIncomingPairing(): string;
  respondIncomingPairing(request: string, allow: boolean): void;
  stopIncomingPairing(): void;
  setReceivingEnabled(enabled: boolean): void;
  getReceivingState(): string;
  getInputReceivingState(): string;
  setInputReceivingEnabled(enabled: boolean): void;
  openInputReceivingSettings(): void;
  resumeReceiving(): void;
  getPairedDevices(): Promise<PairedDevice[]>;
  reserveManualPairing(): string;
  startManualPairing(id: string, address: string, code: string, name: string): Promise<string>;
  getManualPairing(id: string): Promise<string>;
  stopManualPairing(id: string): void;
  pairDevice(
    address: string,
    code: string,
    name: string,
  ): Promise<PairedDevice>;
  startControl(peer: string): Promise<string>;
  stopControl(id: string): void;
  controlEvent(id: string, event: string): void;
  getDeviceAvailability(peer: string): Promise<DeviceAvailability>;
  checkDevice(peer: string): Promise<{ rttMs: number }>;
  forgetDevice(peer: string): Promise<void>;
  getRemovedPeers(): Promise<{ peer: string; name: string }[]>;
  dismissRemovedPeer(peer: string): Promise<string>;
  getDeviceName(): string;
  getLaunchSession(): LaunchSession;
  getMediaState(): string;
  getDisplayReceiverSettings(id: string): Promise<string>;
  requestDisplayReceiverSettings(id: string, expectedRevision: number, settings: string): Promise<string>;
  startScreenSharing(
    peer: string,
  ): Promise<{ started: boolean; error: string }>;
  getScreenSharingState(): string;
  getScreenSharingPreferences(peer: string): string;
  setScreenSharingPreferences(peer: string, value: string): void;
  updateScreenSharingPreferences(
    peer: string,
    value: string,
  ): Promise<{ applied: boolean; error: string; preferences: string }>;
  stopScreenSharing(): void;
  clearLaunchSession(id: string | null): void;
  setImmersive(enabled: boolean): void;
  getAppearance(): "system" | "light" | "dark";
  setAppearance(value: "system" | "light" | "dark"): void;
}>("ExtendDisplay");

export type DisplayInputStatus = {
  state:
    | "disabled"
    | "unsupported"
    | "requesting"
    | "granted"
    | "denied"
    | "suspended";
  reason?: string;
  shortcutPolicy?: "command" | "control" | "" | null;
  shortcutToken?: string | null;
};
export type EditingShortcut = "select_all" | "copy" | "paste" | "undo";
export type NativeInputActions = { editingShortcut(token: string, action: EditingShortcut): Promise<boolean> };
export const NativeDisplay: ComponentType<
  ViewProps & {
    ref?: Ref<NativeInputActions>;
    sessionId?: string;
    touchEnabled?: boolean;
    keyboardEnabled?: boolean;
    onKeyboardStatus?(event: { nativeEvent: { requested: boolean; visible: boolean; error?: string } }): void;
    onInputStatus?(event: { nativeEvent: DisplayInputStatus }): void;
    onStatus(event: { nativeEvent: DisplayStatus }): void;
  }
> = requireNativeViewManager("ExtendDisplay");

export type ControlStatus = {
  state: "connecting" | "connected" | "closed" | "failed";
  error?: string;
  inputError?: string;
  shortcutPolicy?: "command" | "control" | "" | null;
  shortcutToken?: string | null;
};
export const NativeControl: ComponentType<
  ViewProps & {
    ref?: Ref<NativeInputActions>;
    sessionId: string;
    keyboard: boolean;
    onStatus(event: { nativeEvent: ControlStatus }): void;
  }
> = requireNativeViewManager("ExtendDisplay", "Control");
