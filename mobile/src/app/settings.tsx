import { Check } from "lucide-react-native";
import { useCallback, useRef, useState } from "react";
import { useFocusEffect } from "expo-router";
import { PermissionsAndroid, Platform, Pressable, ScrollView, Switch, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Header } from "../components/Header";
import { LocalDeviceSettings } from "../components/LocalDeviceSettings";
import { useTheme } from "../theme";
import { display, type ReceivingState } from "../../modules/extend-display";

export default function Settings() {
  const { appearance, setAppearance, colors } = useTheme();
  const [receiving, setReceiving] = useState<ReceivingState>({ enabled: false, state: "stopped", port: 0 });
  const [input, setInput] = useState({enabled:false,permission:false,active:false,error:""});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const pending = useRef(false);
  const focused = useRef(false);
  useFocusEffect(useCallback(() => {
    focused.current = true;
    const refresh = () => {
      try { setReceiving(JSON.parse(display.getReceivingState())); setInput(JSON.parse(display.getInputReceivingState())); }
      catch { setError("Couldn't check availability. Close and reopen the app to retry."); }
    };
    refresh(); const timer = setInterval(refresh, 1000);
    return () => { focused.current = false; clearInterval(timer); };
  }, []));
  async function toggle(enabled: boolean) {
    if (pending.current) return;
    pending.current = true; setBusy(true); setError(undefined);
    try {
      if (enabled && Platform.OS === "android" && Number(Platform.Version) >= 33)
        await PermissionsAndroid.request(PermissionsAndroid.PERMISSIONS.POST_NOTIFICATIONS);
      if (!focused.current) return;
      display.setReceivingEnabled(enabled);
      setReceiving(JSON.parse(display.getReceivingState()));
    } catch { if (focused.current) setError("Couldn't change availability. Try again while the app is open."); }
    finally { pending.current = false; if (focused.current) setBusy(false); }
  }
  return <SafeAreaView className="flex-1 bg-canvas">
    <Header title="Settings" back />
    <ScrollView contentContainerClassName="mx-auto w-full max-w-3xl gap-4 px-5 py-6" keyboardShouldPersistTaps="handled">
      <Text className="text-sm font-medium text-muted">Connections</Text>
      <View className="gap-3 rounded-2xl border border-line bg-surface p-5">
        <View className="flex-row items-center gap-4">
          <View className="flex-1 gap-1">
            <Text className="text-base font-medium text-ink">Stay available</Text>
            <Text className="text-sm leading-6 text-muted">Allow paired devices to reconnect while this app is in the background.</Text>
          </View>
          <Switch accessibilityLabel="Stay available" disabled={busy} value={receiving.enabled} onValueChange={value => void toggle(value)} />
        </View>
        <Text accessibilityLiveRegion="polite" className="text-sm text-muted">{receiving.state === "available" ? "Available" : receiving.state === "starting" ? "Starting…" : receiving.enabled ? "Not running. Turn off and on to retry." : "Off"}</Text>
        {error ? <Text accessibilityRole="alert" className="text-sm text-muted">{error}</Text> : null}
      </View>
      <View className="gap-3 rounded-2xl border border-line bg-surface p-5">
        <View className="flex-row items-center gap-4">
          <View className="flex-1 gap-1">
            <Text className="text-base font-medium text-ink">Receive keyboard &amp; mouse</Text>
            <Text className="text-sm leading-6 text-muted">Let paired devices control this screen. Stay available must also be on.</Text>
          </View>
          <Switch accessibilityLabel="Receive keyboard and mouse" value={input.enabled} onValueChange={value => {
            try { display.setInputReceivingEnabled(value); setInput(JSON.parse(display.getInputReceivingState())); }
            catch { setError("Couldn't change input receiving."); }
          }} />
        </View>
        <Text accessibilityLiveRegion="polite" className="text-sm text-muted">{input.error || (!input.enabled ? "Off" : !input.permission ? "Accessibility permission needed" : !receiving.enabled ? "Turn on Stay available" : input.active ? "A paired device is controlling this screen" : "Ready for paired devices")}</Text>
        {input.enabled && !input.permission ? <Pressable accessibilityRole="button" accessibilityLabel="Allow input accessibility" onPress={() => {
          try { display.openInputReceivingSettings(); } catch { setError("Open Android Accessibility settings and enable extend.computer input."); }
        }} className="self-start rounded-xl bg-focus px-4 py-3"><Text className="font-medium text-canvas">Open accessibility settings</Text></Pressable> : null}
      </View>
      <Text className="text-sm font-medium text-muted">Appearance</Text>
      <View className="overflow-hidden rounded-2xl border border-line bg-surface">
        {([['system', 'Use device setting'], ['light', 'Light'], ['dark', 'Dark']] as const).map(([value, label], index) =>
          <Pressable key={value} onPress={() => setAppearance(value)} accessibilityRole="radio"
            accessibilityState={{ checked: appearance === value }}
            className={`min-h-16 flex-row items-center justify-between px-5 active:bg-line ${index ? 'border-t border-line' : ''}`}>
            <Text className="text-base text-ink">{label}</Text>
            {appearance === value && <Check size={21} color={colors.focus} />}
          </Pressable>
        )}
      </View>
      <LocalDeviceSettings />
    </ScrollView>
  </SafeAreaView>;
}
