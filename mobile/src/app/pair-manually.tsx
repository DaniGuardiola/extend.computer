import { useCallback, useEffect, useRef, useState } from "react";
import { router, useFocusEffect } from "expo-router";
import {
  ActivityIndicator,
  AppState,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  ScrollView,
  Text,
  TextInput,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { display } from "../../modules/extend-display";
import { Header } from "../components/Header";
import { useTheme } from "../theme";
import { ManualPairingAttempt, type ManualPairingState } from "../pairing/manual";

export default function PairManually() {
  const { colors } = useTheme();
  const [name, setName] = useState("Other device");
  const [address, setAddress] = useState("");
  const [code, setCode] = useState("");
  const [attempt] = useState(() => new ManualPairingAttempt(display));
  const [operationId, setOperationId] = useState<string>();
  const [phase, setPhase] = useState<ManualPairingState["state"]>();
  const busy = !!operationId;
  const [error, setError] = useState<string>();
  const active = useRef(true);
  const generation = useRef(0);
  const canceledOnLeave = useRef(false);
  useFocusEffect(
    useCallback(() => {
      active.current = true;
      generation.current++;
      setOperationId(undefined);
      if (canceledOnLeave.current) {
        canceledOnLeave.current = false;
        setCode("");
        setError("Pairing canceled. Show a new code on the other device before retrying.");
      }
      const cancel = () => {
        generation.current++;
        try { attempt.stop(); } catch { /* Native expiry also fails closed. */ }
        setOperationId(undefined);
        setCode("");
      };
      const lifecycle = AppState.addEventListener("change", value => { if (value !== "active") cancel(); });
      const blur = AppState.addEventListener("blur", cancel);
      return () => {
        generation.current++;
        active.current = false;
        lifecycle.remove(); blur.remove();
        canceledOnLeave.current = !!attempt.id;
        try { attempt.stop(); } catch { /* Keep stale completion out of this screen. */ }
      };
    }, [attempt]),
  );
  const update = useCallback((state: ManualPairingState | null, epoch: number) => {
    if (!state || !active.current || generation.current !== epoch) return;
    setPhase(state.state);
    if (state.state === "paired") {
      setOperationId(undefined); setCode(""); router.replace("/");
    } else if (["failed", "expired", "closed"].includes(state.state)) {
      setOperationId(undefined); setCode("");
      setError(state.state === "expired" ? "Pairing expired. Show a new code on the other device and retry." :
        "Couldn't pair. Check the address and show a new code on the other device, then retry.");
    }
  }, []);
  useEffect(() => {
    if (!operationId) return;
    const epoch = generation.current;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try { update(await attempt.read(), epoch); }
      catch {
        if (!stopped && active.current && generation.current === epoch) {
          setOperationId(undefined); setCode("");
          setError("Couldn't check pairing. Show a new code on the other device and retry.");
        }
      }
      if (!stopped && attempt.id === operationId) timer = setTimeout(() => void poll(), 200);
    };
    void poll();
    return () => { stopped = true; clearTimeout(timer); };
  }, [attempt, operationId, update]);
  async function pair() {
    if (attempt.id || !active.current || AppState.currentState !== "active") return;
    const epoch = generation.current;
    setError(undefined);
    setPhase("starting");
    try {
      const started = attempt.start(address.trim(), code, name.trim());
      setOperationId(attempt.id);
      update(await started, epoch);
    } catch {
      if (active.current && generation.current === epoch) {
        setOperationId(undefined);
        setError(
          "Couldn't pair. Check the address and show a new code on the other device, then retry.",
        );
      }
    }
  }
  function cancel() {
    generation.current++;
    try { attempt.stop(); } catch { /* The bounded native attempt also expires. */ }
    setOperationId(undefined); setCode("");
    setError("Pairing canceled. Show a new code on the other device before retrying.");
  }
  const enabled =
    name.trim().length > 0 &&
    address.trim().length > 0 &&
    /^[0-9a-f]{16}$/i.test(code.replace(/[\s-]/g, ""));
  return (
    <SafeAreaView className="flex-1 bg-canvas">
      <Header title="Pair manually" back />
      <KeyboardAvoidingView
        className="flex-1"
        behavior={Platform.OS === "ios" ? "padding" : undefined}
      >
        <ScrollView
          keyboardShouldPersistTaps="handled"
          contentContainerClassName="mx-auto w-full max-w-xl gap-5 px-5 py-6"
        >
          <Text className="text-sm leading-6 text-muted">
            On your other device, choose Pair device → Pair manually → Show my
            code. Both devices must be reachable on the same network.
          </Text>
          <Pressable
            accessibilityRole="button"
            onPress={() => router.push("/pairing-code")}
            disabled={busy}
            className="min-h-12 self-start justify-center rounded-xl px-3 active:bg-line"
          >
            <Text className="text-sm font-medium text-ink">Show my code</Text>
          </Pressable>
          <View className="gap-2">
            <Text
              nativeID="device-name-label"
              className="text-sm font-medium text-ink"
            >
              Device name
            </Text>
            <TextInput
              accessibilityLabel="Device name"
              editable={!busy}
              value={name}
              onChangeText={setName}
              maxLength={100}
              placeholder="Other device"
              placeholderTextColor={colors.muted}
              className="rounded-xl border border-line bg-surface px-4 py-4 text-base text-ink"
            />
          </View>
          <View className="gap-2">
            <Text className="text-sm font-medium text-ink">Device address</Text>
            <TextInput
              accessibilityLabel="Device address"
              editable={!busy}
              value={address}
              onChangeText={setAddress}
              maxLength={64}
              autoCorrect={false}
              autoCapitalize="none"
              placeholder="192.168.1.20:48177"
              placeholderTextColor={colors.muted}
              className="rounded-xl border border-line bg-surface px-4 py-4 text-base text-ink"
            />
          </View>
          <View className="gap-2">
            <Text className="text-sm font-medium text-ink">Pairing code</Text>
            <TextInput
              accessibilityLabel="Pairing code"
              editable={!busy}
              value={code}
              onChangeText={setCode}
              maxLength={32}
              autoCorrect={false}
              autoCapitalize="none"
              placeholder="xxxx-xxxx-xxxx-xxxx"
              placeholderTextColor={colors.muted}
              className="rounded-xl border border-line bg-surface px-4 py-4 text-base text-ink"
              onSubmitEditing={() => {
                if (enabled) void pair();
              }}
            />
          </View>
          {error ? (
            <Text
              accessibilityRole="alert"
              className="text-sm leading-6 text-muted"
            >
              {error}
            </Text>
          ) : null}
          {busy && <Text accessibilityLiveRegion="polite" className="text-sm leading-6 text-muted">
            {phase === "waiting" ? "Waiting for approval on the other device…" : "Connecting to the other device…"}
          </Text>}
          <Pressable
            accessibilityRole="button"
            accessibilityState={{ disabled: !enabled || busy, busy }}
            disabled={!enabled || busy}
            onPress={() => void pair()}
            className={`min-h-12 flex-row items-center justify-center gap-3 rounded-xl bg-ink px-5 py-4 ${!enabled || busy ? "opacity-50" : "active:opacity-80"}`}
          >
            {busy ? <ActivityIndicator color={colors.canvas} /> : null}
            <Text className="text-base font-medium text-canvas">
              {busy ? "Pairing…" : "Pair device"}
            </Text>
          </Pressable>
          {busy && <Pressable accessibilityRole="button" accessibilityLabel="Cancel pairing" onPress={cancel}
            className="min-h-12 items-center justify-center rounded-xl px-5 py-3 active:bg-line">
            <Text className="text-base font-medium text-ink">Cancel pairing</Text>
          </Pressable>}
        </ScrollView>
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}
