import { useCallback, useEffect, useRef, useState } from "react";
import { AppState, BackHandler, Pressable, Text, View, useWindowDimensions } from "react-native";
import { router, useFocusEffect } from "expo-router";
import { StatusBar } from "expo-status-bar";
import { SafeAreaView } from "react-native-safe-area-context";
import { ArrowLeft, Expand, Hand, Keyboard, Menu, Settings, X } from "lucide-react-native";
import {
  display,
  NativeDisplay,
  type DisplayInputStatus,
  type DisplayStatus,
  type NativeInputActions,
} from "../../modules/extend-display";
import { reconnectOffer } from "../display/offer";
import { displayEndMessage } from "../display/end-message";
import { ReceiverSettingsSheet } from "../components/ReceiverSettingsSheet";
import { EditingShortcuts } from "../components/EditingShortcuts";

export default function Display() {
  const { width } = useWindowDimensions();
  const compactControls = width < 480;
  const [session, setSession] = useState(() => display.getLaunchSession());
  const sessionRef = useRef(session);
  const [status, setStatus] = useState<DisplayStatus>({ state: "connecting" });
  const statusMessage = status.state === "disconnected" ? displayEndMessage(status.message) : status.message;
  const [input, setInput] = useState<DisplayInputStatus>({ state: "disabled" });
  const inputView = useRef<NativeInputActions>(null);
  const [touchEnabled, setTouchEnabled] = useState(false);
  const [keyboardEnabled, setKeyboardEnabled] = useState(false);
  const [keyboardError, setKeyboardError] = useState("");
  const keyboardVisible = useRef(false);
  const [controls, setControls] = useState(true);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const settingsVisible = useRef(false);
  const closeSettings = () => { settingsVisible.current = false; setSettingsOpen(false); };
  useFocusEffect(useCallback(() => {
    if (status.state !== "disconnected") return;
    const timer = setInterval(() => {
      if (AppState.currentState !== "active" || sessionRef.current.id !== session.id) return;
      try {
        const media = JSON.parse(display.getMediaState());
        if (media.state !== "offered") return;
        const offered = reconnectOffer(session.id, media, display.getLaunchSession());
        if (!offered) return;
        // Gate queued callbacks from the previous native view immediately,
        // before React commits the keyed replacement and its cleanup.
        sessionRef.current = offered;
        setTouchEnabled(false);
        setKeyboardEnabled(false);
        keyboardVisible.current = false;
        setKeyboardError("");
        setInput({ state: "disabled" });
        setControls(true);
        settingsVisible.current = false;
        setSettingsOpen(false);
        setStatus({ state: "connecting" });
        setSession(offered);
      } catch { /* Receiving can restart while the terminal notice stays visible. */ }
    }, 300);
    return () => clearInterval(timer);
  }, [session.id, status.state]));
  const leave = () => {
    display.clearLaunchSession(session.id ?? null);
    router.replace("/");
  };
  useEffect(() => {
    display.setImmersive(true);
    const back = BackHandler.addEventListener("hardwareBackPress", () => {
      if (settingsVisible.current) {
        settingsVisible.current = false;
        setSettingsOpen(false);
        return true;
      }
      if (keyboardVisible.current) {
        setKeyboardEnabled(false);
        return true;
      }
      display.clearLaunchSession(session.id ?? null);
      router.replace("/");
      return true;
    });
    return () => {
      back.remove();
      display.setImmersive(false);
    };
  }, [session.id]);
  useEffect(() => {
    if (
      status.state !== "streaming" ||
      !controls ||
      settingsOpen ||
      keyboardEnabled || ((touchEnabled || keyboardEnabled) && input.state !== "granted")
    )
      return;
    const timer = setTimeout(() => setControls(false), 3500);
    return () => clearTimeout(timer);
  }, [status.state, controls, touchEnabled, keyboardEnabled, input.state, settingsOpen]);
  const touchUnavailable =
    !session.id || (input.state === "unsupported" && !touchEnabled);
  const keyboardUnavailable = !keyboardEnabled &&
    (!session.id || input.state === "unsupported" || status.state !== "streaming");
  const touchHint =
    input.state === "granted"
      ? keyboardEnabled
        ? "Text and physical shortcuts · Requires a compatible peer."
        : "Tap to click · Hold to drag · Two fingers to scroll or right-click"
      : input.state === "requesting"
        ? "Requesting input control…"
        : input.state === "denied"
          ? input.reason === "receiving_off"
            ? "Enable Allow connections on the other device, then try touch again."
            : input.reason === "permission"
              ? "Allow input control on the other device, then try touch again."
              : "Input control was declined. Turn touch or keyboard off and on to retry."
          : input.state === "unsupported"
            ? "This display supports viewing only."
            : input.state === "suspended" && (touchEnabled || keyboardEnabled)
              ? "Input paused. Turn touch or keyboard off and on to retry."
              : "Touch control is off.";
  const hideControlsButton = <Pressable
    onPress={() => setControls(false)}
    accessibilityRole="button"
    accessibilityLabel="Hide display controls"
    className="h-12 w-12 items-center justify-center rounded-full active:bg-white/20"
  ><Expand size={20} color="white" /></Pressable>;
  return (
    <View className="flex-1 bg-black">
      <StatusBar hidden />
      {session.available && (
        <NativeDisplay
          ref={inputView}
          key={session.id ?? "anonymous"}
          sessionId={session.id}
          touchEnabled={touchEnabled && !settingsOpen}
          keyboardEnabled={keyboardEnabled && !settingsOpen}
          style={{ flex: 1 }}
          onStatus={(event) => {
            if (sessionRef.current.id === session.id) setStatus(event.nativeEvent);
          }}
          onInputStatus={(event) => {
            if (sessionRef.current.id === session.id) setInput(event.nativeEvent);
          }}
          onKeyboardStatus={(event) => {
            if (sessionRef.current.id !== session.id) return;
            keyboardVisible.current = event.nativeEvent.visible;
            setKeyboardEnabled(event.nativeEvent.requested);
            setKeyboardError(event.nativeEvent.error ?? "");
          }}
        />
      )}
      {controls && (
        <SafeAreaView
          edges={["top"]}
          pointerEvents="box-none"
          style={{ position: "absolute", top: 0, left: 0, right: 0 }}
        >
          <View className="mx-4 mt-3 gap-1 rounded-2xl bg-black/80 px-3 py-2">
            <View className="flex-row flex-wrap items-center gap-2">
              <Pressable
                onPress={leave}
                accessibilityLabel="Stop display session"
                accessibilityRole="button"
                className="h-12 w-12 items-center justify-center rounded-full active:bg-white/20"
              >
                <ArrowLeft size={22} color="white" />
              </Pressable>
              <View className="min-w-0 flex-1">
                <Text
                  numberOfLines={1}
                  className="text-base font-medium text-white"
                >
                  {session.host}
                </Text>
                <Text className="text-xs text-white/70">
                  {session.mode === "mirror"
                    ? "Mirroring screen"
                    : "Extended display"}
                </Text>
              </View>
              {compactControls ? hideControlsButton : null}
              <View className={compactControls ? "w-full flex-row flex-wrap items-center gap-2" : "flex-row items-center gap-2"}>
              <Pressable
                onPress={() => setTouchEnabled((value) => !value)}
                accessibilityRole="switch"
                accessibilityLabel="Touch control"
                accessibilityState={{
                  checked: touchEnabled,
                  disabled: touchUnavailable || status.state !== "streaming",
                }}
                disabled={touchUnavailable || status.state !== "streaming"}
                className={`h-12 flex-row items-center gap-2 rounded-xl px-3 ${touchEnabled ? "bg-white/20" : "active:bg-white/20"} ${touchUnavailable ? "opacity-50" : ""}`}
              >
                <Hand size={18} color="white" />
                <Text className="text-sm text-white">
                  {input.state === "granted"
                    ? "Touch on"
                    : touchEnabled
                      ? input.state === "denied"
                        ? "Touch denied"
                        : input.state === "suspended"
                          ? "Touch paused"
                          : "Touch pending"
                      : "Touch off"}
                </Text>
              </Pressable>
              <Pressable
                onPress={() => setKeyboardEnabled((value) => !value)}
                accessibilityRole="switch"
                accessibilityLabel="Keyboard input"
                accessibilityState={{ checked: keyboardEnabled, disabled: keyboardUnavailable }}
                disabled={keyboardUnavailable}
                className={`h-12 w-12 items-center justify-center rounded-xl ${keyboardEnabled ? "bg-white/20" : "active:bg-white/20"}`}
              >
                <Keyboard size={20} color="white" />
              </Pressable>
              <Pressable
                onPress={() => {
                  setKeyboardEnabled(false);
                  keyboardVisible.current = false;
                  settingsVisible.current = true;
                  setSettingsOpen(true);
                  setControls(true);
                }}
                disabled={!session.id}
                accessibilityRole="button"
                accessibilityLabel="Display settings"
                accessibilityState={{ disabled:!session.id }}
                className="h-12 w-12 items-center justify-center rounded-xl active:bg-white/20 disabled:opacity-50"
              >
                <Settings size={20} color="white" />
              </Pressable>
              {!compactControls ? hideControlsButton : null}
              </View>
            </View>
            {keyboardEnabled && input.shortcutPolicy ? <EditingShortcuts inverse token={input.shortcutToken}
              onAction={async action => !!(sessionRef.current.id === session.id && input.shortcutToken && inputView.current &&
                await inputView.current.editingShortcut(input.shortcutToken, action))} /> : null}
            <Text
              accessibilityLiveRegion="polite"
              className="px-3 pb-2 text-xs leading-5 text-white/70"
            >
              {keyboardError || touchHint}
            </Text>
          </View>
        </SafeAreaView>
      )}
      {!controls && (
        <SafeAreaView
          edges={["bottom", "right"]}
          pointerEvents="box-none"
          style={{ position: "absolute", bottom: 0, right: 0 }}
        >
          <Pressable
            accessibilityLabel="Show display controls"
            accessibilityRole="button"
            onPress={() => setControls(true)}
            className="m-3 h-12 w-12 items-center justify-center rounded-full bg-black/60 active:bg-black/80"
          >
            <Menu size={22} color="white" />
          </Pressable>
        </SafeAreaView>
      )}
      {status.state !== "streaming" && (
        <View
          pointerEvents="box-none"
          style={{ position: "absolute", inset: 0 }}
          className="items-center justify-center gap-4 px-8"
        >
          <Text className="text-xl font-medium text-white">
            {status.state === "connecting"
              ? "Connecting to your display…"
              : statusMessage ? "Display disconnected" : "Display ended"}
          </Text>
          {statusMessage && (
            <Text className="max-w-lg text-center text-sm leading-6 text-white/70">
              {statusMessage}
            </Text>
          )}
          {status.state === "disconnected" && (
            <Pressable
              onPress={leave}
              accessibilityRole="button"
              className="mt-3 flex-row items-center gap-2 rounded-xl bg-white px-5 py-3"
            >
              <X size={18} color="black" />
              <Text className="font-medium text-black">Back to devices</Text>
            </Pressable>
          )}
        </View>
      )}
      {settingsOpen && session.id ? <ReceiverSettingsSheet key={session.id} sessionId={session.id} onClose={closeSettings} /> : null}
    </View>
  );
}
