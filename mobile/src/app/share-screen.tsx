import { useEffect, useRef, useState } from "react";
import { useLocalSearchParams } from "expo-router";
import { Pressable, ScrollView, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { display } from "../../modules/extend-display";
import { Header } from "../components/Header";

type Preferences = { resolution: "fit" | "1280" | "1920"; fps: number; quality: "responsive" | "balanced" | "detail"; codec: "h264" | "hevc" };
function Choices<T extends string | number>({ label, value, options, disabled, onChange }: {
  label: string; value: T; options: { value: T; label: string }[]; disabled: boolean; onChange(value: T): void;
}) {
  return <View className="gap-2">
    <Text className="text-sm font-medium text-muted">{label}</Text>
    <View className="flex-row flex-wrap gap-2">
      {options.map(option => <Pressable key={option.value} accessibilityRole="radio" accessibilityLabel={`${label} ${option.label}`}
        accessibilityState={{ checked: option.value === value, disabled }} disabled={disabled} onPress={() => onChange(option.value)}
        className={`min-h-12 justify-center rounded-xl border px-4 ${option.value === value ? "border-focus bg-focus" : "border-line bg-canvas"} disabled:opacity-50`}>
        <Text className={`text-sm font-medium ${option.value === value ? "text-canvas" : "text-ink"}`}>{option.label}</Text>
      </Pressable>)}
    </View>
  </View>;
}

type Sharing = { state: string; peer?: string; error?: string };
export default function ShareScreen() {
  const { peer, name } = useLocalSearchParams<{ peer: string; name: string }>();
  const [sharing, setSharing] = useState<Sharing>({ state: "idle" });
  const pending = useRef(false);
  const [preferences, setPreferences] = useState<Preferences>(() => {
    try { return JSON.parse(display.getScreenSharingPreferences(peer)); }
    catch { return { resolution: "1280", fps: 60, quality: "balanced", codec: "h264" }; }
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  useEffect(() => {
    const refresh = () => {
      try { setSharing(JSON.parse(display.getScreenSharingState())); }
      catch { setError("Couldn't check screen sharing."); }
    };
    refresh(); const timer = setInterval(refresh, 500);
    return () => clearInterval(timer);
  }, []);
  const active = ["consent", "connecting", "streaming"].includes(sharing.state);
  const settingsDisabled = busy || (active && sharing.state !== "streaming");
  async function start() {
    if (pending.current || active || !peer) return;
    pending.current = true;
    setBusy(true); setError(undefined);
    try { const result = await display.startScreenSharing(peer); if (!result.started) setError(result.error); setSharing(JSON.parse(display.getScreenSharingState())); }
    catch { setError("Couldn't start sharing. Check the other device is available and try again."); }
    finally { pending.current = false; setBusy(false); }
  }
  async function change(update: Partial<Preferences>) {
    const next = { ...preferences, ...update };
    if (pending.current || Object.entries(update).every(([key,value]) => preferences[key as keyof Preferences] === value)) return;
    pending.current = true; setBusy(true); setError(undefined);
    try {
      if (sharing.state === "streaming") {
        const result = await display.updateScreenSharingPreferences(peer, JSON.stringify(next));
        if (!result.applied) { setError(result.error); return; }
        setPreferences(JSON.parse(result.preferences));
      } else {
        display.setScreenSharingPreferences(peer, JSON.stringify(next)); setPreferences(next);
      }
    } catch { setError("Couldn't update screen sharing settings. Try again."); }
    finally { pending.current = false; setBusy(false); }

  }
  return <SafeAreaView className="flex-1 bg-canvas">
    <Header title="Share screen" back />
    <ScrollView contentContainerClassName="mx-auto w-full max-w-3xl gap-5 px-5 py-6">
      <View className="gap-3 rounded-2xl border border-line bg-surface p-5">
        <Text className="text-lg font-semibold text-ink">{name || "Paired device"}</Text>
        <Text className="text-sm leading-6 text-muted">Show your screen on this device. Android will ask which screen or app to share.</Text>
        <Choices label="Resolution" value={preferences.resolution} disabled={settingsDisabled} onChange={resolution => void change({ resolution })}
          options={[{value:"1280",label:"1280 px"},{value:"1920",label:"1920 px"},{value:"fit",label:"Fit screen"}]} />
        <Choices label="Frame rate" value={preferences.fps} disabled={settingsDisabled} onChange={fps => void change({ fps })}
          options={[30,60,120].map(value => ({value,label:`${value} FPS`}))} />
        <Choices label="Video quality" value={preferences.quality} disabled={settingsDisabled} onChange={quality => void change({ quality })}
          options={[{value:"responsive",label:"Lower bandwidth · 8 Mbps"},{value:"balanced",label:"Balanced · 20 Mbps"},{value:"detail",label:"Detail · 40 Mbps"}]} />
        <Choices label="Video codec" value={preferences.codec} disabled={active || busy} onChange={codec => void change({ codec })}
          options={[{value:"h264",label:"H.264"},{value:"hevc",label:"HEVC"}]} />
        {active ? <Text className="text-sm text-muted">Stop sharing to change the video codec.</Text> : null}
        {busy && sharing.state === "streaming" ? <Text accessibilityLiveRegion="polite" className="text-sm text-muted">Applying settings…</Text> : null}
        <Text accessibilityLiveRegion="polite" className="text-sm text-ink">{sharing.state === "streaming" ? "Sharing your screen" : sharing.state === "connecting" ? "Connecting…" : sharing.state === "consent" ? "Choose what to share in Android’s dialog" : "Screen sharing is off"}</Text>
        {error || sharing.error ? <Text accessibilityRole="alert" className="text-sm text-muted">{error || sharing.error}</Text> : null}
        {active ? <Pressable accessibilityRole="button" accessibilityLabel="Stop sharing" onPress={() => {
          try { display.stopScreenSharing(); } catch { setError("Couldn't stop sharing. Use Android’s screen sharing indicator to stop."); }
        }} className="min-h-12 items-center justify-center rounded-xl bg-focus px-4"><Text className="font-medium text-canvas">Stop sharing</Text></Pressable>
          : <Pressable accessibilityRole="button" accessibilityLabel="Start sharing" disabled={busy || !peer} onPress={() => void start()}
            className="min-h-12 items-center justify-center rounded-xl bg-focus px-4 disabled:opacity-50"><Text className="font-medium text-canvas">{busy ? "Connecting…" : "Start sharing"}</Text></Pressable>}
      </View>
    </ScrollView>
  </SafeAreaView>;
}
