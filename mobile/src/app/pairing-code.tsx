import { useCallback, useRef, useState } from "react";
import { router, useFocusEffect } from "expo-router";
import { Pressable, ScrollView, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import {
  display,
  type IncomingPairingState,
} from "../../modules/extend-display";
import { Header } from "../components/Header";
import { PairingVerification } from "../components/PairingVerification";

export default function PairingCode() {
  const [pairing, setPairing] = useState<IncomingPairingState>();
  const [error, setError] = useState<string>();
  const answered = useRef<string | undefined>(undefined);
  const [answerId, setAnswerId] = useState<string>();
  const start = useCallback(() => {
    setError(undefined);
    answered.current = undefined;
    setAnswerId(undefined);
    try {
      setPairing(JSON.parse(display.startIncomingPairing()));
    } catch {
      setError("Couldn't open pairing. Close and reopen this screen to retry.");
    }
  }, []);
  useFocusEffect(
    useCallback(() => {
      start();
      const timer = setInterval(() => {
        try {
          setPairing(JSON.parse(display.getIncomingPairing()));
        } catch {
          setError(
            "Couldn't check pairing. Close and reopen this screen to retry.",
          );
        }
      }, 500);
      return () => {
        clearInterval(timer);
        display.stopIncomingPairing();
      };
    }, [start]),
  );
  function respond(allow: boolean) {
    if (!pairing?.request || answered.current === pairing.request.id) return;
    try {
      display.respondIncomingPairing(pairing.request.id, allow);
      answered.current = pairing.request.id;
      setAnswerId(pairing.request.id);
      setPairing(JSON.parse(display.getIncomingPairing()));
    } catch {
      setError("This request expired. Show a new code and retry.");
    }
  }
  const request = pairing?.request;
  const terminal =
    pairing &&
    ["paired", "rejected", "expired", "closed"].includes(pairing.state);
  return (
    <SafeAreaView className="flex-1 bg-canvas">
      <Header title="Show my code" back />
      <ScrollView contentContainerClassName="mx-auto w-full max-w-xl gap-6 px-5 py-6">
        <Text className="text-base leading-6 text-muted">
          On your other device, choose Pair device → Pair manually and enter
          this code. Select this tablet nearby to fill its address, or enter
          one of the addresses shown here on your other device.
        </Text>
        {pairing?.code ? (
          <View className="gap-3 rounded-2xl border border-line bg-surface p-6">
            <Text className="text-sm font-medium text-muted">Pairing code</Text>
            <Text
              selectable
              accessibilityLabel={`Pairing code ${pairing.code}`}
              className="text-center text-2xl font-semibold tracking-wider text-ink"
            >
              {pairing.code}
            </Text>
            <Text
              accessibilityLiveRegion="polite"
              className="text-center text-sm text-muted"
            >
              Expires in {pairing.secondsRemaining} seconds
            </Text>
          </View>
        ) : null}
        {pairing?.code && pairing.addresses.length > 0 ? (
          <View className="gap-2">
            <Text className="text-sm text-muted">
              If this tablet does not appear nearby, enter an address:
            </Text>
            {pairing.addresses.map((address) => (
              <Text
                key={address}
                selectable
                accessibilityLabel={`Pairing address ${address}`}
                className="text-sm text-ink"
              >
                {address}
              </Text>
            ))}
          </View>
        ) : null}
        {request?.kind === "verify" ? (
          <PairingVerification
            symbols={request.symbols}
            busy={answerId === request.id}
            respond={respond}
          />
        ) : request ? (
          <View className="gap-4 rounded-2xl border border-line bg-surface p-6">
            <Text className="text-lg font-semibold text-ink">
              Allow pairing?
            </Text>
            <Text className="text-sm leading-6 text-muted">
              A device entered your code. Allow only if you just entered it on
              your other device.
            </Text>
            <Text selectable className="text-xs text-muted">
              Device ID: {request.peer}
            </Text>
            <View className="flex-row gap-3">
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Deny pairing"
                disabled={answerId === request.id}
                onPress={() => respond(false)}
                className="min-h-12 flex-1 items-center justify-center rounded-xl bg-canvas px-4 active:bg-line"
              >
                <Text className="text-base font-medium text-ink">Deny</Text>
              </Pressable>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Allow pairing"
                disabled={answerId === request.id}
                onPress={() => respond(true)}
                className="min-h-12 flex-1 items-center justify-center rounded-xl bg-ink px-4 active:opacity-80"
              >
                <Text className="text-base font-medium text-canvas">Allow</Text>
              </Pressable>
            </View>
          </View>
        ) : null}
        {terminal ? (
          <View className="gap-4">
            <Text
              accessibilityLiveRegion="polite"
              className="text-base text-ink"
            >
              {pairing.state === "paired"
                ? "Device paired"
                : pairing.state === "rejected"
                  ? "Pairing wasn't approved"
                  : "Pairing closed"}
            </Text>
            {pairing.state === "paired" ? (
              <Pressable
                accessibilityRole="button"
                onPress={() => router.replace("/")}
                className="min-h-12 items-center justify-center rounded-xl bg-ink px-4 active:opacity-80"
              >
                <Text className="text-base font-medium text-canvas">Done</Text>
              </Pressable>
            ) : (
              <Pressable
                accessibilityRole="button"
                onPress={start}
                className="min-h-12 items-center justify-center rounded-xl bg-ink px-4 active:opacity-80"
              >
                <Text className="text-base font-medium text-canvas">
                  Show new code
                </Text>
              </Pressable>
            )}
          </View>
        ) : null}
        {error ? (
          <Text accessibilityRole="alert" className="text-sm text-muted">
            {error}
          </Text>
        ) : null}
        <Text className="text-sm leading-6 text-muted">
          Keep this screen open while pairing. Leaving the app closes the code.
        </Text>
      </ScrollView>
    </SafeAreaView>
  );
}
