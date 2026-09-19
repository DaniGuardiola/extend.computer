import { useCallback, useEffect, useRef, useState } from "react";
import { api, native, type Permissions } from "./bridge";

// One owner for startup/focus checks, setup polling, and explicit pre-connect checks.
export function usePermissions(
  watch: boolean,
  onChange: (value: Permissions) => void,
) {
  const [checked, setChecked] = useState(false);
  const [checkError, setCheckError] = useState("");
  const [guideError, setGuideError] = useState("");
  const mounted = useRef(false);
  const pending = useRef<Promise<Permissions> | null>(null);
  const check = useCallback(() => {
    if (pending.current) return pending.current;
    pending.current = api
      .checkPermissions()
      .then((value) => {
        if (mounted.current) {
          onChange(value);
          setChecked(true);
          setCheckError("");
        }
        return value;
      })
      .catch((error) => {
        if (mounted.current) setCheckError(String(error));
        throw error;
      })
      .finally(() => {
        pending.current = null;
      });
    return pending.current;
  }, [onChange]);

  useEffect(() => {
    mounted.current = true;
    const refresh = () => {
      if (native) void check().catch(() => {});
    };
    refresh();
    window.addEventListener("focus", refresh);
    return () => {
      mounted.current = false;
      window.removeEventListener("focus", refresh);
    };
  }, [check]);

  useEffect(() => {
    if (!native || !watch) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      await check().catch(() => {});
      if (!stopped) timer = setTimeout(poll, 2000);
    };
    void poll();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [watch, check]);

  const open = async (permission: "listen" | "post" | "wifi") => {
    setGuideError("");
    try {
      await api.openPermission(permission);
      await check();
    } catch (error) {
      if (mounted.current) setGuideError(String(error));
    }
  };
  return { checked, error: guideError || checkError, check, open };
}
