import { useEffect, useRef, useState } from "react";
import { api, type LocalDeviceInfo } from "./bridge";

export function DeviceNameSettings() {
  const inputRef = useRef<HTMLInputElement>(null);
  const nameRowRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const row = nameRowRef.current;
    if (!row) return;
    const preventBackgroundSelection = (event: Event) => {
      // WebKit can start selection on user-select:none background during a
      // tiny drag, then move focus back into the adjacent input. Cancel that
      // selection at its source; editing and selection inside the input stay native.
      if (event.target !== inputRef.current) event.preventDefault();
    };
    row.addEventListener("selectstart", preventBackgroundSelection);
    return () =>
      row.removeEventListener("selectstart", preventBackgroundSelection);
  }, []);
  const [info, setInfo] = useState<LocalDeviceInfo | null>(null);
  const [name, setName] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const skipBlur = useRef(false);
  const pending = useRef(false);
  useEffect(() => {
    void api
      .localDeviceInfo()
      .then((value) => {
        setInfo(value);
        setName(value.name);
      })
      .catch((error) => setError(String(error)));
  }, []);
  const save = async () => {
    if (!info || pending.current || name.trim() === info.name) return;
    pending.current = true;
    setSaving(true);
    setError("");
    try {
      await api.setLocalDeviceName(name);
      const value = await api.localDeviceInfo();
      setInfo(value);
      setName(value.name);
    } catch (error) {
      setError(String(error));
    } finally {
      pending.current = false;
      setSaving(false);
    }
  };
  return (
    <section className="mb-8" aria-labelledby="device-name-heading">
      <div ref={nameRowRef} className="flex items-center justify-between gap-4">
        <h2 id="device-name-heading" className="shrink-0 text-sm font-medium">
          Device name
        </h2>
        <input
          ref={inputRef}
          aria-label="Device name"
          aria-busy={saving}
          aria-invalid={!!error}
          title="Edit name. Enter to save, Escape to cancel. Leave blank to use this Mac’s system name."
          placeholder={info ? "System name" : "Loading…"}
          disabled={!info}
          readOnly={saving}
          value={name}
          onChange={(event) => setName(event.target.value)}
          onFocus={() => {
            skipBlur.current = false;
          }}
          onBlur={() => {
            if (!skipBlur.current) void save();
            skipBlur.current = false;
          }}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing) return;
            if (event.key === "Enter") {
              event.preventDefault();
              event.currentTarget.blur();
            }
            if (event.key === "Escape") {
              event.preventDefault();
              skipBlur.current = true;
              setName(info?.name ?? "");
              setError("");
              event.currentTarget.blur();
            }
          }}
          className="min-w-0 w-64 max-w-[65%] rounded border border-transparent bg-transparent px-2 py-1.5 text-right text-sm text-ink hover:border-line focus:border-line focus:outline-none"
        />
      </div>
      {error && (
        <p className="error mt-2 text-xs" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
