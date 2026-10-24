import { useCallback, useEffect, useRef, useState } from "react";
import { useFocusEffect } from "expo-router";
import { AppState } from "react-native";
import { display, type PairedDevice } from "../../modules/extend-display";
import { deviceRequests, type DeviceStatus } from "./availability";

export function useDevices() {
  const [devices, setDevices] = useState<PairedDevice[]>([]);
  const [statuses, setStatuses] = useState<Record<string, DeviceStatus>>({});
  const [focused, setFocused] = useState(false);
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const [version, setVersion] = useState(0);
  const [now, setNow] = useState(() => Date.now());
  const [error, setError] = useState<string>();
  const generation = useRef(0);
  const foregroundState = useRef(AppState.currentState === "active");
  const manual = useRef<string | undefined>(undefined);
  const active = focused && foreground;
  useFocusEffect(
    useCallback(() => {
      setStatuses({});
      setFocused(true);
      return () => {
        setFocused(false);
        generation.current++;
      };
    }, []),
  );
  useEffect(() => {
    const listener = AppState.addEventListener("change", (state) => {
      const next = state === "active";
      if (foregroundState.current === next) return;
      foregroundState.current = next;
      generation.current++;
      setStatuses({});
      setForeground(next);
    });
    return () => listener.remove();
  }, []);
  useEffect(() => {
    const epoch = ++generation.current;
    if (!active) {
      manual.current = undefined;
      return;
    }
    let timer: ReturnType<typeof setTimeout> | undefined;
    let stopped = false;
    const valid = () => !stopped && generation.current === epoch;
    const clock = setInterval(() => {
      if (valid()) setNow(Date.now());
    }, 1000);
    async function scan() {
      try {
        const peers = await display.getPairedDevices();
        if (!valid()) return;
        setDevices(peers);
        setError(undefined);
        for (const peer of peers) {
          if (!valid()) return;
          const requested = manual.current;
          const selected =
            peers.find((item) => item.peer === requested) ?? peer;
          try {
            const result = await deviceRequests.run(async () => {
              if (!valid()) throw new Error("Status screen closed");
              if (requested === selected.peer) {
                if (manual.current === requested) manual.current = undefined;
                try {
                  await display.checkDevice(selected.peer);
                } catch {
                  /* The read-only status below distinguishes receiving-off and unreachable peers. */
                }
              }
              if (!valid()) throw new Error("Status screen closed");
              return display.getDeviceAvailability(selected.peer);
            });
            if (!valid()) return;
            if (result.kind !== "busy") {
              const status =
                result.kind === "done"
                  ? result.value
                  : {
                      state: "unavailable" as const,
                      reason: "Status check timed out. Try again.",
                    };
              const checkedAt = Date.now();
              setNow(checkedAt);
              setStatuses((old) => ({
                ...old,
                [selected.peer]: { ...status, checkedAt },
              }));
            }
          } catch {
            if (!valid()) return;
            setStatuses((old) => ({
              ...old,
              [selected.peer]: {
                state: "unavailable",
                checkedAt: Date.now(),
                reason:
                  "Couldn't connect. Open the other device and check Allow connections.",
              },
            }));
          }
        }
      } catch {
        if (valid())
          setError(
            "Couldn't load your devices. Close and reopen the app to retry.",
          );
      } finally {
        if (valid())
          timer = setTimeout(() => void scan(), manual.current ? 1000 : 5000);
      }
    }
    void scan();
    return () => {
      stopped = true;
      if (timer) clearTimeout(timer);
      clearInterval(clock);
    };
  }, [active, version]);
  const check = (peer: string) => {
    if (!active) return;
    manual.current = peer;
    setStatuses((old) => ({
      ...old,
      [peer]: { state: "checking", checkedAt: Date.now() },
    }));
    setVersion((value) => value + 1);
  };
  const reload = () => setVersion((value) => value + 1);
  return { devices, statuses, now, error, check, reload, active };
}
