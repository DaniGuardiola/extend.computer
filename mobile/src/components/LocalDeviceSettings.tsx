import { useCallback, useState } from "react";
import { Text, View } from "react-native";
import { display } from "../../modules/extend-display";
import { useDeviceSetting } from "../devices/useDeviceSetting";
import { DeviceNameEditor } from "./DeviceNameEditor";

export function LocalDeviceSettings() {
  const read = useCallback(() => display.getLocalDeviceInfo(), []);
  const { value: info, loading, working, error, save } = useDeviceSetting(read);
  const [draft, setDraft] = useState<string>();
  async function saveName() {
    if (draft !== undefined && await save(() => display.setLocalDeviceName(draft))) setDraft(undefined);
  }
  return <>
    <Text className="text-sm font-medium text-muted">This device</Text>
    <View className="gap-3 rounded-2xl border border-line bg-surface p-5">
      <Text className="text-base font-medium text-ink">Device name</Text>
      <Text className="text-sm leading-6 text-muted">The name other devices see when connecting.</Text>
      {info ? <DeviceNameEditor name={info.name} defaultName={info.defaultName} custom={info.customName} draft={draft}
        onChange={setDraft} onSave={() => void saveName()} onDiscard={() => setDraft(undefined)} busy={working || loading} resetLabel="Use system name" />
        : <Text accessibilityLiveRegion="polite" className="text-sm text-muted">{loading ? "Loading device…" : "Device information unavailable"}</Text>}
      {error ? <Text accessibilityRole="alert" className="text-sm leading-6 text-red-600">{error}</Text> : null}
    </View>
    <Text className="mt-5 text-sm font-medium text-muted">About</Text>
    <View className="gap-3 rounded-2xl border border-line bg-surface p-5">
      <Text className="text-base font-semibold text-ink">extend.computer</Text>
      {info ? <>
        <Text className="text-sm text-muted">Version {info.version || "unknown"} · Build {info.build}</Text>
        <Text className="text-sm font-medium text-ink">Device fingerprint</Text>
        <Text selectable className="font-mono text-xs leading-5 text-ink">{info.fingerprint}</Text>
        <Text className="text-sm leading-6 text-muted">This public identifier verifies your device when pairing.</Text>
      </> : <Text className="text-sm text-muted">Device information unavailable</Text>}
    </View>
  </>;
}
