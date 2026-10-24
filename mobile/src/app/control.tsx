import { useEffect, useRef, useState } from "react";
import { useLocalSearchParams } from "expo-router";
import { AppState, Pressable, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Keyboard, MousePointer2 } from "lucide-react-native";
import { display, NativeControl, type ControlStatus, type NativeInputActions } from "../../modules/extend-display";
import { EditingShortcuts } from "../components/EditingShortcuts";
import { Header } from "../components/Header";
import { useTheme } from "../theme";

export default function Control() {
  const { peer, name } = useLocalSearchParams<{ peer: string; name: string }>();
  const { colors } = useTheme();
  const [id, setId] = useState<string>();
  const [status, setStatus] = useState<ControlStatus>({ state: "connecting" });
  const [keyboard, setKeyboard] = useState(false);
  const inputView = useRef<NativeInputActions>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let stopped = false;
    let session: string | undefined;
    const stop = () => { stopped = true; if (session) display.stopControl(session); };
    const lifecycle = AppState.addEventListener("change", state => {
      if (state !== "active") { stop(); setKeyboard(false); setStatus({ state: "closed", error: "Control stopped when the app left the screen." }); }
    });
    display.startControl(typeof peer === "string" ? peer : "").then(value => {
      const initial = JSON.parse(value) as { id: string };
      session = initial.id;
      if (stopped || AppState.currentState !== "active") display.stopControl(session);
      else setId(session);
    }).catch(() => { if (!stopped) setStatus({ state: "failed", error: "Couldn't start control. Check the other device is accepting connections and has input permissions." }); });
    return () => { lifecycle.remove(); stop(); };
  }, [peer, attempt]);
  const connected = status.state === "connected";
  function event(value: object) {
    if (!id || !connected) return;
    try { display.controlEvent(id, JSON.stringify(value)); }
    catch { setStatus({ state: "failed", error: "Control ended. Reconnect to try again." }); }
  }
  function click(button: number) {
    event({ kind: "button", button, down: true, clicks: 1 });
    event({ kind: "button", button, down: false, clicks: 1 });
  }
  function key(code: number) {
    event({ kind: "mac_key", code, down: true, repeat: false });
    event({ kind: "mac_key", code, down: false, repeat: false });
  }
  return <SafeAreaView className="flex-1 bg-canvas">
    <Header title={name || "Keyboard & mouse"} back />
    <View className="mx-auto w-full max-w-5xl flex-1 gap-4 px-5 pb-5">
      <Text accessibilityLiveRegion="polite" className="text-sm text-muted">
        {connected ? "Controlling the main screen" : status.state === "connecting" ? "Connecting…" : "Control stopped"}
      </Text>
      <View className="flex-1 overflow-hidden rounded-2xl border border-line bg-surface">
        {id && <NativeControl ref={inputView} sessionId={id} keyboard={keyboard} style={{ flex: 1 }} onStatus={({ nativeEvent: value }) => setStatus(previous => value.inputError ? { ...previous, inputError: value.inputError } : value)} />}
        <View pointerEvents="none" className="items-center justify-center gap-3 px-6" style={{ position: "absolute", inset: 0 }}>
          <MousePointer2 size={28} color={colors.muted} />
          <Text className="text-center text-base font-medium text-muted">{connected ? "Move one finger · Tap to click" : status.state === "connecting" ? "Preparing trackpad…" : "Trackpad disconnected"}</Text>
          {connected && <Text className="text-center text-sm text-muted">Two fingers to scroll · Hold, then move to drag</Text>}
        </View>
      </View>
      {(status.error || status.inputError) && <Text accessibilityRole="alert" className="text-sm text-muted">{status.error || status.inputError}</Text>}
      {connected ? <View className="flex-row flex-wrap gap-3">
        <Pressable accessibilityRole="button" accessibilityLabel="Remote keyboard" accessibilityState={{ selected: keyboard }} onPress={() => setKeyboard(value => !value)}
          className="min-h-12 flex-row items-center gap-2 rounded-xl border border-line bg-surface px-4 active:bg-line">
          <Keyboard size={20} color={colors.ink} /><Text className="font-medium text-ink">Keyboard</Text>
        </Pressable>
        <Pressable accessibilityRole="button" accessibilityLabel="Remote right click" onPress={() => click(1)} className="min-h-12 justify-center rounded-xl border border-line bg-surface px-4 active:bg-line"><Text className="text-ink">Right click</Text></Pressable>
        <Pressable accessibilityRole="button" accessibilityLabel="Remote tab" onPress={() => key(48)} className="min-h-12 justify-center rounded-xl border border-line bg-surface px-4 active:bg-line"><Text className="text-ink">Tab</Text></Pressable>
        <Pressable accessibilityRole="button" accessibilityLabel="Remote escape" onPress={() => key(53)} className="min-h-12 justify-center rounded-xl border border-line bg-surface px-4 active:bg-line"><Text className="text-ink">Esc</Text></Pressable>
      </View> : status.state !== "connecting" && <Pressable accessibilityRole="button" onPress={() => { setId(undefined); setKeyboard(false); setStatus({ state: "connecting" }); setAttempt(value => value+1); }} className="min-h-12 items-center justify-center rounded-xl bg-ink px-4"><Text className="font-medium text-surface">Reconnect</Text></Pressable>}
      {connected && keyboard && status.shortcutPolicy ? <EditingShortcuts token={status.shortcutToken}
        onAction={async action => !!(status.shortcutToken && inputView.current &&
          await inputView.current.editingShortcut(status.shortcutToken, action))} /> : null}
    </View>
  </SafeAreaView>;
}
