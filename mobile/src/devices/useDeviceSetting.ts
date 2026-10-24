import { useCallback, useRef, useState } from "react";
import { useFocusEffect } from "expo-router";
import { AppState } from "react-native";

/** Read on focus/resume; native settings never poll or accept an old screen's result. */
export function useDeviceSetting<T>(read: () => Promise<T>) {
  const [value, setValue] = useState<T>();
  const [loading, setLoading] = useState(true);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<string>();
  const focused = useRef(false);
  const generation = useRef(0);
  const reading = useRef(false);
  const pending = useRef(false);
  const latestRefresh = useRef<() => Promise<void>>(async () => {});
  const visible = useCallback(() => focused.current && AppState.currentState === "active", []);
  const refresh = useCallback(async (): Promise<void> => {
    if (!visible() || reading.current || pending.current) return;
    const epoch = generation.current;
    reading.current = true;
    setLoading(true);
    try {
      const next = await read();
      if (visible() && generation.current === epoch) { setValue(next); setError(undefined); }
    } catch {
      if (visible() && generation.current === epoch) setError("Couldn't load this device. Reopen this screen to retry.");
    } finally {
      reading.current = false;
      if (visible()) {
        if (generation.current === epoch) setLoading(false);
        else void latestRefresh.current();
      }
    }
  }, [read, visible]);
  useFocusEffect(useCallback(() => {
    latestRefresh.current = refresh;
    focused.current = true;
    generation.current++;
    setWorking(pending.current);
    void refresh();
    const listener = AppState.addEventListener("change", state => {
      generation.current++;
      if (state === "active") void refresh();
    });
    return () => { focused.current = false; generation.current++; listener.remove(); };
  }, [refresh]));
  const save = async (operation: () => Promise<T>, failureMessage = "Couldn't save this name. Use a shorter name without line breaks and try again."): Promise<boolean> => {
    if (!visible() || pending.current || reading.current) return false;
    const epoch = generation.current;
    pending.current = true;
    setWorking(true);
    setError(undefined);
    let saved = false;
    try {
      const next = await operation();
      if (visible() && generation.current === epoch) { setValue(next); saved = true; }
    } catch {
      if (visible() && generation.current === epoch)
        setError(failureMessage);
    } finally {
      pending.current = false;
      if (visible()) {
        setWorking(false);
        if (generation.current !== epoch) void latestRefresh.current();
      }
    }
    return saved;
  };
  const clearError = useCallback(() => setError(undefined), []);
  return { value, loading, working, error, save, refresh, clearError };
}
