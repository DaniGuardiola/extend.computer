import { useCallback, useRef, useState } from "react";
import { router, useFocusEffect } from "expo-router";
import { ChevronRight, Laptop } from "lucide-react-native";
import { AppState, Pressable, ScrollView, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import {
  display,
  type DiscoverySnapshot,
  type IncomingPairingState,
  type VisualPairingState,
} from "../../modules/extend-display";
import { Header } from "../components/Header";
import { PairingVerification } from "../components/PairingVerification";
import { useTheme } from "../theme";

function stopVisual(id: string) {
  try {
    display.stopVisualPairing(id);
  } catch {
    /* Background cancellation may already have removed this exact operation. */
  }
}

export default function PairDevice() {
  const { colors } = useTheme();
  const [nearby, setNearby] = useState<DiscoverySnapshot>({
    state: "idle",
    candidates: [],
  });
  const [incoming, setIncoming] = useState<IncomingPairingState>();
  const [outgoing, setOutgoing] = useState<VisualPairingState>();
  const [error, setError] = useState<string>();
  const [starting, setStarting] = useState(false);
  const [answered, setAnswered] = useState<string>();
  const [connectionActive, setConnectionActive] = useState(false);
  const [pairedAddresses, setPairedAddresses] = useState<string[]>([]);
  const generation = useRef(0);
  const visualId = useRef<string | undefined>(undefined);
  const pending = useRef(false);
  const attempt = useRef(0);
  const answeredId = useRef<string | undefined>(undefined);
  useFocusEffect(
    useCallback(() => {
      const epoch = ++generation.current;
      setStarting(pending.current);
      setError(undefined);
      setOutgoing(undefined);
      answeredId.current = undefined;
      setAnswered(undefined);
      try {
        const connected = display.hasActiveConnection();
        setConnectionActive(connected);
        if (!connected) {
          display.startDiscovery();
          setIncoming(JSON.parse(display.startIncomingPairing()));
        }
      } catch {
        setError("Couldn't open nearby pairing. Try again, or pair manually.");
      }
      display
        .getPairedDevices()
        .then((peers) => {
          if (generation.current === epoch)
            setPairedAddresses(peers.map((peer) => peer.address));
        })
        .catch(() => {});
      const lifecycle = AppState.addEventListener("change", (state) => {
        if (state === "active") return;
        attempt.current++;
        if (visualId.current) stopVisual(visualId.current);
        visualId.current = undefined;
        setOutgoing(undefined);
        setStarting(false);
        setError(undefined);
        answeredId.current = undefined;
        setAnswered(undefined);
        setIncoming({
          state: "closed",
          port: 0,
          code: null,
          secondsRemaining: 0,
          addresses: [],
          request: null,
        });
      });
      const refresh = () => {
        if (AppState.currentState !== "active" || generation.current !== epoch)
          return;
        try {
          setConnectionActive(display.hasActiveConnection());
          setNearby(display.getNearbyDevices());
          setIncoming(JSON.parse(display.getIncomingPairing()));
          if (visualId.current)
            setOutgoing(JSON.parse(display.getVisualPairing(visualId.current)));
        } catch {
          setError(
            "Couldn't check pairing. Close and reopen this screen to retry.",
          );
        }
      };
      refresh();
      const timer = setInterval(refresh, 500);
      return () => {
        generation.current++;
        attempt.current++;
        lifecycle.remove();
        clearInterval(timer);
        display.stopDiscovery();
        display.stopIncomingPairing();
        if (visualId.current) {
          stopVisual(visualId.current);
          visualId.current = undefined;
        }
      };
    }, []),
  );
  async function pair(address: string, name: string) {
    if (
      pending.current ||
      visualId.current ||
      incoming?.request ||
      display.hasActiveConnection()
    )
      return;
    const epoch = generation.current;
    const ticket = attempt.current;
    pending.current = true;
    setStarting(true);
    setError(undefined);
    try {
      const state = JSON.parse(
        await display.startVisualPairing(address, name),
      ) as VisualPairingState;
      if (generation.current !== epoch || attempt.current !== ticket) {
        stopVisual(state.id);
        return;
      }
      visualId.current = state.id;
      setOutgoing(state);
    } catch {
      if (generation.current === epoch && attempt.current === ticket)
        setError("Couldn't start nearby pairing. Retry or pair manually.");
    } finally {
      pending.current = false;
      if (generation.current === epoch) setStarting(false);
    }
  }
  function respond(allow: boolean) {
    const request = outgoing?.request ?? incoming?.request;
    if (!request || answeredId.current === request.id) return;
    try {
      if (outgoing?.request)
        display.respondVisualPairing(outgoing.id, request.id, allow);
      else display.respondIncomingPairing(request.id, allow);
      answeredId.current = request.id;
      setAnswered(request.id);
    } catch {
      setError("This request expired. Cancel pairing and retry.");
    }
  }
  function cancel() {
    if (visualId.current) {
      stopVisual(visualId.current);
      visualId.current = undefined;
    }
    setOutgoing(undefined);
    answeredId.current = undefined;
    setAnswered(undefined);
    setError(undefined);
  }
  const request = outgoing?.request ?? incoming?.request;
  const success = outgoing?.state === "paired" || incoming?.state === "paired";
  const busy = connectionActive || starting || !!outgoing || !!request;
  const awaitingIncoming =
    !!answered && incoming?.state === "waiting" && !incoming.code;
  const focusedPairing =
    !success &&
    (starting ||
      !!request ||
      awaitingIncoming ||
      outgoing?.state === "connecting" ||
      outgoing?.state === "verify");
  const candidates = nearby.candidates.filter(
    (candidate) =>
      candidate.addresses.length &&
      !candidate.addresses.some(
        (address) =>
          pairedAddresses.includes(address) ||
          incoming?.addresses.includes(address),
      ),
  );
  const terminal =
    outgoing &&
    ["rejected", "failed", "expired", "closed"].includes(outgoing.state);
  return (
    <SafeAreaView className="flex-1 bg-canvas">
      <Header title="Pair a device" back />
      <ScrollView contentContainerClassName="mx-auto w-full max-w-xl gap-5 px-5 py-6">
        {connectionActive && !focusedPairing ? (
          <Text className="text-sm leading-6 text-muted">
            Stop the current connection before pairing another device.
          </Text>
        ) : null}
        {!focusedPairing ? (
          <Text
            accessibilityLiveRegion="polite"
            className="text-sm leading-6 text-muted"
          >
            {success
              ? "Device paired"
              : incoming?.code
                ? "Your device is visible nearby."
                : "Nearby pairing is closed. Reopen this screen to make this device visible."}
          </Text>
        ) : null}
        {!incoming?.code &&
        !focusedPairing &&
        !request &&
        !outgoing &&
        !success &&
        !connectionActive ? (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Retry nearby pairing"
            onPress={() => {
              try {
                display.startDiscovery();
                setIncoming(JSON.parse(display.startIncomingPairing()));
                answeredId.current = undefined;
                setAnswered(undefined);
                setError(undefined);
              } catch {
                setError("Couldn't reopen pairing. Retry or pair manually.");
              }
            }}
            className="min-h-12 self-start justify-center rounded-xl px-3 active:bg-line"
          >
            <Text className="font-medium text-ink">Retry nearby pairing</Text>
          </Pressable>
        ) : null}
        {!success && request?.kind === "verify" ? (
          <PairingVerification
            symbols={request.symbols}
            busy={answered === request.id}
            respond={respond}
          />
        ) : !success && request ? (
          <View className="gap-3 rounded-xl border border-line bg-surface p-5">
            <Text className="text-base font-medium text-ink">
              A device entered your pairing code
            </Text>
            <Text className="text-sm leading-6 text-muted">
              Cancel this request unless you entered this tablet’s code on your
              other device.
            </Text>
            <View className="flex-row gap-3">
              <Pressable
                accessibilityRole="button"
                disabled={answered === request.id}
                onPress={() => respond(false)}
                className="min-h-12 flex-1 items-center justify-center rounded-xl bg-canvas"
              >
                <Text className="font-medium text-ink">Deny</Text>
              </Pressable>
              <Pressable
                accessibilityRole="button"
                disabled={answered === request.id}
                onPress={() => respond(true)}
                className="min-h-12 flex-1 items-center justify-center rounded-xl bg-ink"
              >
                <Text className="font-medium text-canvas">Allow</Text>
              </Pressable>
            </View>
          </View>
        ) : null}
        {success ? (
          <Pressable
            accessibilityRole="button"
            onPress={() => router.replace("/")}
            className="min-h-12 items-center justify-center rounded-xl bg-ink"
          >
            <Text className="font-medium text-canvas">Done</Text>
          </Pressable>
        ) : focusedPairing ? (
          <>
            {!request || answered === request.id ? (
              <Text
                accessibilityLiveRegion="polite"
                className="text-sm leading-6 text-muted"
              >
                Waiting for the other device to confirm…
              </Text>
            ) : null}
            {outgoing ? (
              <Pressable
                accessibilityRole="button"
                onPress={cancel}
                className="min-h-12 self-start justify-center rounded-xl px-3 active:bg-line"
              >
                <Text className="font-medium text-ink">Cancel pairing</Text>
              </Pressable>
            ) : null}
          </>
        ) : (
          <>
            <Text className="text-sm font-medium text-muted">
              Nearby devices
            </Text>
            {candidates.map((candidate) => (
              <Pressable
                key={candidate.id}
                disabled={busy}
                accessibilityRole="button"
                accessibilityLabel={`Pair with ${candidate.name}`}
                onPress={() =>
                  void pair(candidate.addresses[0], candidate.name)
                }
                className={`min-h-16 flex-row items-center gap-3 rounded-xl border border-line bg-surface px-4 py-4 ${busy ? "opacity-50" : "active:bg-line"}`}
              >
                <Laptop size={21} color={colors.muted} />
                <Text className="flex-1 text-base text-ink">
                  {candidate.name}
                </Text>
                <ChevronRight size={18} color={colors.muted} />
              </Pressable>
            ))}
            {!candidates.length ? (
              <Text className="py-5 text-center text-sm leading-6 text-muted">
                {nearby.state === "failed"
                  ? "Couldn't search nearby. Try pairing manually."
                  : "Open Pair device on your other device."}
              </Text>
            ) : null}
            {terminal ? (
              <Text
                accessibilityLiveRegion="polite"
                className="text-sm text-muted"
              >
                {outgoing.error ||
                  (outgoing.state === "rejected"
                    ? "Pairing wasn't approved."
                    : "Pairing closed. Retry or pair manually.")}
              </Text>
            ) : null}
            {outgoing ? (
              <Pressable
                accessibilityRole="button"
                onPress={cancel}
                className="min-h-12 self-start justify-center rounded-xl px-3 active:bg-line"
              >
                <Text className="font-medium text-ink">
                  {terminal ? "Try again" : "Cancel pairing"}
                </Text>
              </Pressable>
            ) : null}
            <View className="border-t border-line pt-4">
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Pair manually"
                disabled={busy}
                onPress={() => router.push("/pair-manually")}
                className={`min-h-12 self-start justify-center rounded-xl px-3 ${busy ? "opacity-50" : "active:bg-line"}`}
              >
                <Text className="font-medium text-ink">Pair manually</Text>
              </Pressable>
            </View>
          </>
        )}
        {error ? (
          <Text
            accessibilityRole="alert"
            className="text-sm leading-6 text-muted"
          >
            {error}
          </Text>
        ) : null}
      </ScrollView>
    </SafeAreaView>
  );
}
