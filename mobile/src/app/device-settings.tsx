import { useCallback, useRef, useState } from "react";
import { useLocalSearchParams, useFocusEffect } from "expo-router";
import { AppState, Pressable, ScrollView, Text, TextInput, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { display } from "../../modules/extend-display";
import { DeviceNameEditor } from "../components/DeviceNameEditor";
import { Header } from "../components/Header";
import { deviceRequests } from "../devices/availability";
import { useDeviceSetting } from "../devices/useDeviceSetting";
import { useTheme } from "../theme";

export default function DeviceSettings() {
  const params = useLocalSearchParams<{ peer: string | string[] }>();
  const peer = Array.isArray(params.peer) ? params.peer[0] : params.peer;
  return <PeerDeviceSettings key={peer ?? ""} peer={peer ?? ""} />;
}

function PeerDeviceSettings({ peer }: { peer: string }) {
  const { colors } = useTheme();
  const read = useCallback(() => display.getPeerDeviceInfo(peer ?? ""), [peer]);
  const { value: info, loading, working, error, save, refresh, clearError } = useDeviceSetting(read);
  const [draft, setDraft] = useState<string>();
  const [advanced, setAdvanced] = useState(false);
  const [addressDraft, setAddressDraft] = useState<string>();
  const [saveKind, setSaveKind] = useState<"name" | "address">("name");
  const settingPending = useRef(false);
  const [checking, setChecking] = useState(false);
  const [status, setStatus] = useState<string>();
  const focused = useRef(false);
  const checkPending = useRef(false);
  const checkGeneration = useRef(0);
  const busy = loading || working || checking;
  const addressDirty = addressDraft !== undefined && addressDraft.trim() !== info?.address;
  useFocusEffect(useCallback(() => {
    focused.current = true;
    checkGeneration.current++;
    setChecking(checkPending.current);
    const listener = AppState.addEventListener("change", () => { checkGeneration.current++; });
    return () => { focused.current = false; checkGeneration.current++; listener.remove(); };
  }, []));
  async function saveName() {
    if (!peer || draft === undefined || busy || settingPending.current || checkPending.current) return;
    settingPending.current = true;
    setSaveKind("name");
    try {
      if (await save(() => display.setPeerDeviceLabel(peer, draft))) setDraft(undefined);
    } finally { settingPending.current = false; }
  }
  async function saveAddress() {
    if (!peer || addressDraft === undefined || !addressDirty || busy || settingPending.current || checkPending.current) return;
    settingPending.current = true;
    setSaveKind("address");
    setStatus(undefined);
    try {
      if (await save(() => display.setPeerDeviceAddress(peer, addressDraft.trim()),
        "Couldn't save this address. Use a numeric IP address and port, such as 192.168.1.20:48177 or [2001:db8::1]:48177.")) setAddressDraft(undefined);
    } finally { settingPending.current = false; }
  }
  async function checkConnection() {
    if (!peer || busy || settingPending.current || checkPending.current) return;
    const epoch = checkGeneration.current;
    const valid = () => focused.current && checkGeneration.current === epoch && AppState.currentState === "active";
    checkPending.current = true; setChecking(true); setStatus(undefined);
    try {
      const result = await deviceRequests.run(async () => {
        if (!valid()) throw new Error("Screen closed");
        try { await display.checkDevice(peer); } catch { /* Read-only availability distinguishes receiving-off from unreachable. */ }
        if (!valid()) throw new Error("Screen closed");
        return display.getDeviceAvailability(peer);
      });
      if (valid()) setStatus(result.kind === "done"
        ? result.value.state === "online" ? "Online" : result.value.state === "receiving_off" ? "Not accepting connections" : "Couldn't connect. Open the other device and check Allow connections."
        : result.kind === "busy" ? "Another device check is running. Try again." : "Connection check timed out. Try again.");
      // A pinned check may repair the stored endpoint. Read it without replacing either draft.
      if (valid() && result.kind === "done") await refresh();
    } catch {
      if (valid()) setStatus("Couldn't connect. Open the other device and check Allow connections.");
    } finally { checkPending.current = false; if (focused.current) setChecking(false); }
  }
  return <SafeAreaView className="flex-1 bg-canvas">
    <Header title="Device settings" back />
    <ScrollView contentContainerClassName="mx-auto w-full max-w-xl gap-5 px-5 py-6" keyboardShouldPersistTaps="handled">
      <View className="gap-3 rounded-2xl border border-line bg-surface p-5">
        <Text className="text-base font-medium text-ink">Device name</Text>
        <Text className="text-sm leading-6 text-muted">Choose how this device appears on this tablet.</Text>
        {info ? <DeviceNameEditor name={info.name} defaultName={info.defaultName ?? info.name} custom={!!info.label} draft={draft}
          onChange={setDraft} onSave={() => void saveName()} onDiscard={() => setDraft(undefined)} busy={busy} resetLabel="Use original name" />
          : <Text accessibilityLiveRegion="polite" className="text-sm text-muted">{loading ? "Loading device…" : "Device unavailable"}</Text>}
        {error && saveKind === "name" ? <Text accessibilityRole="alert" className="text-sm leading-6 text-red-600">{error}</Text> : null}
      </View>
      {info ? <View className="gap-3 rounded-2xl border border-line bg-surface p-5">
        <Text className="text-base font-medium text-ink">Identity</Text>
        {info.authenticatedName ? <Text className="text-sm text-muted">Reported name: {info.authenticatedName}</Text> : null}
        <Text className="text-sm text-muted">{info.trustSource === "account" ? "Connected through your account" : "Paired directly"}</Text>
        <Text selectable className="font-mono text-xs leading-5 text-ink">{info.peer}</Text>
        <Text className="text-sm leading-6 text-muted">Changing the name here preserves this device&apos;s identity.</Text>
        <Pressable accessibilityRole="button" accessibilityLabel="Check connection" disabled={busy}
          accessibilityState={{ disabled: busy }} onPress={() => void checkConnection()}
          className={`min-h-12 self-start justify-center rounded-xl px-4 ${busy ? "opacity-50" : "active:bg-line"}`}>
          <Text className="text-sm font-medium text-ink">{checking ? "Checking…" : "Check connection"}</Text>
        </Pressable>
        {status ? <Text accessibilityLiveRegion="polite" className="text-sm leading-6 text-muted">{status}</Text> : null}
      </View> : null}
      {info ? <View className="gap-3 rounded-2xl border border-line bg-surface p-5">
        <Pressable accessibilityRole="button" accessibilityLabel="Advanced settings" disabled={busy}
          accessibilityState={{ expanded: advanced, disabled: busy }} onPress={() => setAdvanced(value => !value)}
          className={`min-h-12 flex-row items-center justify-between gap-3 ${busy ? "opacity-50" : "active:opacity-70"}`}>
          <Text className="text-base font-medium text-ink">Advanced</Text>
          <Text className="text-sm text-muted">{advanced ? "Hide" : "Show"}</Text>
        </Pressable>
        {addressDirty ? <Text accessibilityLiveRegion="polite" className="text-xs text-muted">Unsaved address changes</Text> : null}
        {advanced ? <View className="gap-3">
          <Text className="text-sm font-medium text-ink">Address</Text>
          <Text className="text-sm leading-6 text-muted">Set an IP address and port when this device cannot be found nearby. Saving preserves its pairing; use Check connection to verify it.</Text>
          <TextInput accessibilityLabel="Device address" editable={!busy} value={addressDraft ?? info.address}
            onChangeText={setAddressDraft} placeholder="192.168.1.20:48177" placeholderTextColor={colors.muted}
            keyboardType="url" autoCapitalize="none" autoCorrect={false} spellCheck={false} autoComplete="off"
            maxLength={128} returnKeyType="done" disableFullscreenUI
            className="min-h-12 rounded-xl border border-line bg-canvas px-4 text-base text-ink" />
          <Text className="text-xs leading-5 text-muted">IPv4:port or [IPv6]:port</Text>
          {addressDirty ? <View className="flex-row flex-wrap gap-2">
            <Pressable accessibilityRole="button" accessibilityLabel="Discard address changes" disabled={busy}
              accessibilityState={{ disabled: busy }} onPress={() => { setAddressDraft(undefined); if (saveKind === "address") clearError(); }}
              className={`min-h-12 justify-center rounded-xl px-4 ${busy ? "opacity-50" : "active:bg-line"}`}>
              <Text className="text-sm font-medium text-ink">Discard</Text>
            </Pressable>
            <Pressable accessibilityRole="button" accessibilityLabel="Save device address" disabled={busy}
              accessibilityState={{ disabled: busy }} onPress={() => void saveAddress()}
              className={`min-h-12 justify-center rounded-xl bg-focus px-5 ${busy ? "opacity-50" : "active:opacity-80"}`}>
              <Text className="text-sm font-semibold text-canvas">{working && saveKind === "address" ? "Saving…" : "Save"}</Text>
            </Pressable>
          </View> : null}
        </View> : null}
        {error && saveKind === "address" ? <Text accessibilityRole="alert" className="text-sm leading-6 text-red-600">{error}</Text> : null}
      </View> : null}
    </ScrollView>
  </SafeAreaView>;
}
